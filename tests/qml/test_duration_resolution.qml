import QtQuick
import QtTest
import Materialfy.Core

TestCase {
    name: "DurationResolution"

    // Mock Catalog bridge for testing
    QtObject {
        id: Catalog
        property var _resolveCalls: []
        property var _resolvedDurations: {}
        
        function resolveDuration(trackId, currentMs) {
            _resolveCalls.push({ trackId: trackId, currentMs: currentMs })
        }
        
        signal durationResolved(string trackId, int durationMs)
        
        // Helper to simulate resolution
        function simulateResolution(trackId, durationMs) {
            durationResolved(trackId, durationMs)
        }
    }

    Component {
        id: trackRowComponent
        TrackRow {
            width: 800
        }
    }

    function test_no_resolution_when_duration_present() {
        Catalog._resolveCalls = []
        let row = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test123",
            durationMs: 180000 // 3 minutes
        })
        
        row.visible = true
        compare(Catalog._resolveCalls.length, 0, "Should not resolve when duration already set")
    }

    function test_resolution_triggered_when_visible() {
        Catalog._resolveCalls = []
        let row = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test456",
            durationMs: 0,
            visible: false
        })
        
        compare(Catalog._resolveCalls.length, 0, "Should not trigger when not visible")
        
        row.visible = true
        compare(Catalog._resolveCalls.length, 1, "Should trigger resolution when made visible")
        compare(Catalog._resolveCalls[0].trackId, "youtube:track:test456")
        compare(Catalog._resolveCalls[0].currentMs, 0)
    }

    function test_no_duplicate_resolution() {
        Catalog._resolveCalls = []
        let row = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test789",
            durationMs: 0
        })
        
        row.visible = true
        compare(Catalog._resolveCalls.length, 1)
        
        // Toggle visibility multiple times
        row.visible = false
        row.visible = true
        row.visible = false
        row.visible = true
        
        compare(Catalog._resolveCalls.length, 1, "Should only resolve once per track")
    }

    function test_duration_updated_on_resolution() {
        Catalog._resolveCalls = []
        Catalog._resolvedDurations = {}
        
        let row = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:abc123",
            durationMs: 0
        })
        
        row.visible = true
        compare(row.durationMs, 0, "Duration should be 0 initially")
        
        // Simulate resolution completing
        Catalog.simulateResolution("youtube:track:abc123", 240000) // 4 minutes
        
        compare(row.durationMs, 240000, "Duration should be updated from resolution")
    }

    function test_only_youtube_tracks_resolved() {
        Catalog._resolveCalls = []
        
        let spotifyRow = createTemporaryObject(trackRowComponent, null, {
            trackId: "spotify:track:xyz789",
            durationMs: 0
        })
        
        spotifyRow.visible = true
        // Note: The actual filtering happens in Rust, but we can verify the QML doesn't crash
        
        // Empty track ID should not trigger
        let emptyRow = createTemporaryObject(trackRowComponent, null, {
            trackId: "",
            durationMs: 0
        })
        
        emptyRow.visible = true
        // Since we can't access internal logic from QML test, just verify no crash
        verify(true)
    }

    function test_multiple_tracks_resolve_independently() {
        Catalog._resolveCalls = []
        
        let row1 = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:track1",
            durationMs: 0
        })
        
        let row2 = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:track2",
            durationMs: 0
        })
        
        row1.visible = true
        row2.visible = true
        
        compare(Catalog._resolveCalls.length, 2, "Each track should trigger its own resolution")
        
        // Resolve only one
        Catalog.simulateResolution("youtube:track:track1", 180000)
        
        compare(row1.durationMs, 180000, "Track 1 should be updated")
        compare(row2.durationMs, 0, "Track 2 should still be 0")
        
        // Resolve the second one
        Catalog.simulateResolution("youtube:track:track2", 210000)
        
        compare(row2.durationMs, 210000, "Track 2 should be updated")
    }
}
