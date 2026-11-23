// Try the intro on its own, the way the panel shows it:
//   quickshell -p intro/shell.qml
import QtQuick
import Quickshell
import Quickshell.Wayland

ShellRoot {
  PanelWindow {
    anchors { top: true; bottom: true; left: true; right: true }
    exclusionMode: ExclusionMode.Ignore
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
    WlrLayershell.namespace: "ebb-intro"
    color: "black"
    Cracktro {
      anchors.fill: parent
      onFinished: Qt.quit()
    }
  }
}
