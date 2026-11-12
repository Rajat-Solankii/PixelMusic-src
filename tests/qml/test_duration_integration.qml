import QtQuick 2.15
import QtTest 1.15
import Materialfy.Core 1.0

TestCase {
    name: "DurationResolutionIntegration"
    
    // Mock Catalog for testing
    QtObject {
        id: mockCatalog
        property var _resolveRequests: []
        signal durationResolved(string trackId, int durationMs)
        
        function resolveDuration(trackId, currentMs) {
            _resolveRequests.push({trackId: trackId, currentMs: currentMs})
        }
    }
    
    // Mock TrackRow component
    Component {
        id: trackRowComponent
        
        QtObject {
            id: trackRow
            property string trackId: ""
            property int durationMs: 0
            property bool visible: false
            property bool _durationResolutionRequested: false
            
            onVisibleChanged: {
                if (visible && durationMs <= 0 && trackId !== "" && !_durationResolutionRequested) {
                    _durationResolutionRequested = true
                    mockCatalog.resolveDuration(trackId, durationMs)
                }
            }
            
            Connections {
                target: mockCatalog
                function onDurationResolved(resolvedTrackId, resolvedMs) {
                    if (resolvedTrackId === trackRow.trackId && trackRow.durationMs <= 0 && resolvedMs > 0) {
                        trackRow.durationMs = resolvedMs
                    }
                }
            }
        }
    }
    
    function test_duration_resolution_requested_once() {
        // Create a track row
        const track = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test123"
        })
        
        // Initially not visible and duration is 0
        compare(track.durationMs, 0, "Initial duration should be 0")
        compare(track._durationResolutionRequested, false, "Should not have requested resolution yet")
        compare(mockCatalog._resolveRequests.length, 0, "No requests yet")
        
        // Make visible - should request resolution
        track.visible = true
        compare(track._durationResolutionRequested, true, "Should have requested resolution")
        compare(mockCatalog._resolveRequests.length, 1, "Should have one request")
        compare(mockCatalog._resolveRequests[0].trackId, "youtube:track:test123", "Track ID should match")
        
        // Toggle visibility again - should not request again
        track.visible = false
        track.visible = true
        compare(mockCatalog._resolveRequests.length, 1, "Should still have only one request")
    }
    
    function test_duration_updates_from_signal() {
        const track = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test456",
            visible: true
        })
        
        // Duration should be 0 initially
        compare(track.durationMs, 0, "Initial duration should be 0")
        
        // Simulate resolution success
        mockCatalog.durationResolved("youtube:track:test456", 240000) // 4 minutes
        
        // Duration should update
        compare(track.durationMs, 240000, "Duration should be updated from signal")
    }
    
    function test_duration_not_updated_for_wrong_track() {
        const track = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test789",
            visible: true
        })
        
        // Try to update with wrong track ID
        mockCatalog.durationResolved("youtube:track:wrong", 180000)
        compare(track.durationMs, 0, "Duration should not update for wrong track ID")
        
        // Correct track ID should update
        mockCatalog.durationResolved("youtube:track:test789", 180000)
        compare(track.durationMs, 180000, "Duration should update for correct track ID")
    }
    
    function test_duration_not_overwritten_if_already_set() {
        // Create track with duration already set
        const track = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test111",
            durationMs: 200000,
            visible: true
        })
        
        // Try to update with different duration - should not overwrite
        mockCatalog.durationResolved("youtube:track:test111", 180000)
        compare(track.durationMs, 200000, "Should not overwrite existing duration")
    }
    
    function test_no_resolution_for_empty_track_id() {
        mockCatalog._resolveRequests = [] // Reset
        
        const track = createTemporaryObject(trackRowComponent, null, {
            trackId: "",
            visible: true
        })
        
        compare(mockCatalog._resolveRequests.length, 0, "Should not request resolution for empty track ID")
    }
    
    function test_no_resolution_if_duration_already_set() {
        mockCatalog._resolveRequests = [] // Reset
        
        const track = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:test222",
            durationMs: 180000,
            visible: true
        })
        
        // Should not request resolution if duration is already set
        compare(mockCatalog._resolveRequests.length, 0, "Should not request resolution if duration already set")
    }
    
    function test_multiple_tracks_resolve_independently() {
        // Create multiple tracks
        const track1 = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:track1",
            visible: true
        })
        const track2 = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:track2",
            visible: true
        })
        const track3 = createTemporaryObject(trackRowComponent, null, {
            trackId: "youtube:track:track3",
            visible: true
        })
        
        // Should have 3 requests
        compare(mockCatalog._resolveRequests.length, 3, "Should have 3 requests")
        
        // Resolve each independently
        mockCatalog.durationResolved("youtube:track:track1", 180000)
        mockCatalog.durationResolved("youtube:track:track2", 240000)
        mockCatalog.durationResolved("youtube:track:track3", 300000)
        
        compare(track1.durationMs, 180000, "Track 1 duration")
        compare(track2.durationMs, 240000, "Track 2 duration")
        compare(track3.durationMs, 300000, "Track 3 duration")
    }
}
