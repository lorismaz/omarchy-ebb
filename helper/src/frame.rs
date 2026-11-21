//! Message framing on the Zepp channel (PROTOCOL.md §2.1) and the AES pieces
//! it and the account login need.

use aes::Aes128;
use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};

const MAGIC: u8 = 0x03;
pub const FIRST: u8 = 0x01;
pub const LAST: u8 = 0x02;
pub const ACK_WANTED: u8 = 0x04;
pub const ENCRYPTED: u8 = 0x08;
const MAX_MESSAGE: usize = 64 * 1024;

pub fn aes_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let mut b = Array::from(*block);
    Aes128::new(&Array::from(*key)).encrypt_block(&mut b);
    b.into()
}

fn aes_ecb(key: &[u8; 16], data: &mut [u8], decrypt: bool) {
    let cipher = Aes128::new(&Array::from(*key));
    for chunk in data.chunks_exact_mut(16) {
        let mut b = Array::try_from(&*chunk).expect("16-byte block");
        if decrypt {
            cipher.decrypt_block(&mut b);
        } else {
            cipher.encrypt_block(&mut b);
        }
        chunk.copy_from_slice(&b);
    }
}

/// AES-128-CBC with PKCS#7 padding.
pub fn aes_cbc_pkcs7(key: &[u8; 16], iv: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    let pad = 16 - plain.len() % 16;
    let mut data = plain.to_vec();
    data.extend(std::iter::repeat_n(pad as u8, pad));
    let mut prev = *iv;
    for chunk in data.chunks_exact_mut(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ prev[i];
        }
        prev = aes_block(key, &block);
        chunk.copy_from_slice(&prev);
    }
    data
}

fn message_key(session: &[u8; 16], handle: u8) -> [u8; 16] {
    session.map(|b| b ^ handle)
}

/// Turns messages into frames for one direction (phone → strap).
#[derive(Default)]
pub struct Sender {
    pub handle: u8,
    pub sequence: u32,
    pub session: Option<[u8; 16]>,
}

impl Sender {
    pub fn frames(&mut self, endpoint: u16, payload: &[u8], encrypt: bool, mtu: usize) -> Vec<Vec<u8>> {
        self.handle = self.handle.wrapping_add(1);
        let body = if encrypt {
            let session = self.session.expect("encrypting before the handshake finished");
            let mut sealed = payload.to_vec();
            sealed.extend_from_slice(&self.sequence.to_le_bytes());
            let crc = crc32fast::hash(&sealed);
            sealed.extend_from_slice(&crc.to_le_bytes());
            sealed.resize(sealed.len().div_ceil(16) * 16, 0);
            aes_ecb(&message_key(&session, self.handle), &mut sealed, false);
            self.sequence = self.sequence.wrapping_add(1);
            sealed
        } else {
            payload.to_vec()
        };

        let room = (mtu.saturating_sub(3)).clamp(20, 512);
        let mut frames = Vec::new();
        let mut rest = &body[..];
        let mut index: u8 = 0;
        loop {
            let first = frames.is_empty();
            let mut frame = vec![MAGIC, 0, 0, self.handle, index];
            if first {
                frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                frame.extend_from_slice(&endpoint.to_le_bytes());
            }
            let take = rest.len().min(room - frame.len());
            frame.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            let mut flags = if encrypt { ENCRYPTED } else { 0 };
            if first {
                flags |= FIRST;
            }
            if rest.is_empty() {
                flags |= LAST | ACK_WANTED;
            }
            frame[1] = flags;
            frames.push(frame);
            index = index.wrapping_add(1);
            if rest.is_empty() {
                return frames;
            }
        }
    }
}

struct Partial {
    handle: u8,
    endpoint: u16,
    length: usize,
    encrypted: bool,
    data: Vec<u8>,
}

/// Reassembles frames from the strap into messages.
#[derive(Default)]
pub struct Receiver {
    pub session: Option<[u8; 16]>,
    partial: Option<Partial>,
}

pub type Message = (u16, Vec<u8>);

impl Receiver {
    /// -> (completed message, ack to send back)
    pub fn push(&mut self, frame: &[u8]) -> (Option<Message>, Option<Vec<u8>>) {
        if frame.len() < 5 || frame[0] != MAGIC {
            return (None, None);
        }
        let (flags, handle, index) = (frame[1], frame[3], frame[4]);
        let ack = (flags & ACK_WANTED != 0).then(|| vec![0x04, 0x00, handle, 0x01, index]);

        if flags & FIRST != 0 {
            self.partial = None;
            if frame.len() < 11 {
                return (None, ack);
            }
            let length = u32::from_le_bytes(frame[5..9].try_into().unwrap()) as usize;
            if length > MAX_MESSAGE {
                return (None, ack);
            }
            self.partial = Some(Partial {
                handle,
                endpoint: u16::from_le_bytes([frame[9], frame[10]]),
                length,
                encrypted: flags & ENCRYPTED != 0,
                data: frame[11..].to_vec(),
            });
        } else {
            match self.partial.as_mut() {
                Some(p) if p.handle == handle => p.data.extend_from_slice(&frame[5..]),
                _ => return (None, ack),
            }
        }

        if self.partial.as_ref().is_some_and(|p| p.data.len() > MAX_MESSAGE + 32) {
            self.partial = None;
            return (None, ack);
        }
        if flags & LAST == 0 {
            return (None, ack);
        }
        let Some(mut p) = self.partial.take() else {
            return (None, ack);
        };
        if p.encrypted {
            let Some(session) = self.session else { return (None, ack) };
            if p.data.len() % 16 != 0 {
                return (None, ack);
            }
            aes_ecb(&message_key(&session, p.handle), &mut p.data, true);
        }
        if p.data.len() < p.length {
            return (None, ack);
        }
        p.data.truncate(p.length);
        (Some((p.endpoint, p.data)), ack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn aes_known_answers() {
        // FIPS-197 appendix C.1
        let key: [u8; 16] = core::array::from_fn(|i| i as u8);
        let ct = aes_block(&key, &hex("00112233445566778899aabbccddeeff").try_into().unwrap());
        assert_eq!(ct.to_vec(), hex("69c4e0d86a7b0430d8cdb78070b4c55a"));
        // NIST SP 800-38A F.2.1, first block, plus a full padding block
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let out = aes_cbc_pkcs7(&key, &iv, &hex("6bc1bee22e409f96e93d7e117393172a"));
        assert_eq!(out[..16].to_vec(), hex("7649abac8119b246cee98e9b12e9197d"));
        assert_eq!(out.len(), 32);
        assert_eq!(aes_cbc_pkcs7(&key, &iv, &[b'x'; 15]).len(), 16);
    }

    #[test]
    fn single_plain_frame() {
        // services request (endpoint 0, payload 03) as the third message
        let mut s = Sender { handle: 2, ..Default::default() };
        assert_eq!(s.frames(0x0000, &[0x03], false, 23), vec![hex("030700030001000000000003")]);
    }

    #[test]
    fn long_plain_message_splits() {
        let mut s = Sender::default();
        let frames = s.frames(0x0082, &[7u8; 52], false, 23);
        assert!(frames.iter().all(|f| f.len() <= 20));
        assert_eq!(frames[0][..11], [3, 1, 0, 1, 0, 52, 0, 0, 0, 0x82, 0]);
        assert_eq!(frames.last().unwrap()[1], LAST | ACK_WANTED);
        let mut r = Receiver::default();
        let got: Vec<_> = frames.iter().filter_map(|f| r.push(f).0).collect();
        assert_eq!(got, vec![(0x0082, vec![7u8; 52])]);
    }

    #[test]
    fn encrypted_round_trip() {
        let session = [9u8; 16];
        let mut s = Sender { session: Some(session), sequence: 41, ..Default::default() };
        let mut r = Receiver { session: Some(session), ..Default::default() };
        for mtu in [23, 247] {
            let msg: Vec<u8> = (0..40).collect();
            let mut out = None;
            let mut last_ack = None;
            for f in s.frames(0x0016, &msg, true, mtu) {
                assert!(f[1] & ENCRYPTED != 0);
                let (m, ack) = r.push(&f);
                out = m.or(out);
                last_ack = ack.or(last_ack);
            }
            assert_eq!(out, Some((0x0016, msg)));
            let ack = last_ack.unwrap();
            assert_eq!((ack[0], ack[1], ack[3]), (4, 0, 1));
        }
        assert_eq!(s.sequence, 43);
    }

    #[test]
    fn junk_and_oversized_claims() {
        let mut r = Receiver::default();
        assert_eq!(r.push(&[0, 1]), (None, None));
        let mut f = vec![3, 1, 0, 9, 0];
        f.extend_from_slice(&1_000_000_000u32.to_le_bytes());
        f.extend_from_slice(&[0x16, 0]);
        assert_eq!(r.push(&f).0, None);
        // an encrypted message before the handshake is dropped
        let mut s = Sender { session: Some([1; 16]), ..Default::default() };
        for frame in s.frames(0x16, b"hi", true, 23) {
            assert_eq!(r.push(&frame).0, None);
        }
    }
}
