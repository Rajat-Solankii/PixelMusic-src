import QtQuick

// Sideways scrolling for a horizontal row (a Flickable or horizontal ListView): a two-finger touchpad swipe left or
// right, or a tilting mouse wheel, moves the row directly under the fingers. Vertical wheel movement is left to the
// page (this handler only takes horizontal deltas). Put it inside the row: SideScroll { flickable: row }.
WheelHandler {
    id: root

    property Flickable flickable: parent as Flickable
    // one notch of a tilting wheel (no pixel delta) moves this far
    property real notchStep: 96

    target: null
    acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
    orientation: Qt.Horizontal

    onWheel: event => {
        const f = root.flickable
        if (!f)
            return
        const dx = event.pixelDelta.x !== 0 ? event.pixelDelta.x : event.angleDelta.x / 120 * root.notchStep
        if (dx === 0)
            return
        const maxX = f.originX + Math.max(0, f.contentWidth - f.width)
        f.contentX = Math.max(f.originX, Math.min(maxX, f.contentX - dx))
    }
}
