import QtQuick 2.15
import QtTest 1.15
import Materialfy.Core 1.0
import "../src/materialfy/qml/app"

TestCase {
    name: "LogoColorwayIntegration"
    
    // Test that LogoPaths responds to Settings changes
    LogoPaths {
        id: logoPaths
    }
    
    function test_default_colorway() {
        // Default should be bluePurple
        compare(logoPaths.currentColorway, "bluePurple", "Default colorway should be bluePurple")
        verify(logoPaths.currentColors !== undefined, "Current colors should be defined")
        compare(logoPaths.currentColors.gradient.length, 2, "Gradient should have 2 colors")
        compare(logoPaths.currentColors.lilac, "#F3EEFF", "Lilac color should match")
        compare(logoPaths.currentColors.ember, "#FF7A4D", "Ember color should match")
    }
    
    function test_colorway_updates() {
        // Skip if Settings not available (test environment)
        if (typeof Settings === 'undefined') {
            skip("Settings singleton not available in test environment")
        }
        
        // Set to a different colorway
        const originalColorway = Settings.logoColorway
        
        try {
            Settings.logoColorway = "graphiteGray"
            
            // Wait for signal propagation
            wait(100)
            
            compare(logoPaths.currentColorway, "graphiteGray", "Colorway should update")
            verify(logoPaths.currentColors !== undefined, "Colors should be defined")
            compare(logoPaths.currentColors.gradient[0], "#8E8E93", "Gradient start should match")
            compare(logoPaths.currentColors.gradient[1], "#636366", "Gradient end should match")
        } finally {
            // Restore original setting
            Settings.logoColorway = originalColorway
        }
    }
    
    function test_all_colorways_defined() {
        // Verify all colorways are properly defined
        const expectedColorways = ["bluePurple", "babyPinkPurple", "graphiteGray", "limeGreen", "limeLemon", "orangeYellow"]
        
        for (const colorway of expectedColorways) {
            verify(logoPaths.colorways[colorway] !== undefined, `Colorway ${colorway} should be defined`)
            verify(logoPaths.colorways[colorway].gradient !== undefined, `Colorway ${colorway} should have gradient`)
            verify(logoPaths.colorways[colorway].lilac !== undefined, `Colorway ${colorway} should have lilac`)
            verify(logoPaths.colorways[colorway].ember !== undefined, `Colorway ${colorway} should have ember`)
        }
    }
    
    function test_invalid_colorway_fallback() {
        // Skip if Settings not available
        if (typeof Settings === 'undefined') {
            skip("Settings singleton not available in test environment")
        }
        
        const originalColorway = Settings.logoColorway
        
        try {
            // Set to an invalid colorway
            Settings.logoColorway = "invalidColor"
            
            // Wait for signal propagation
            wait(100)
            
            // Should fallback to bluePurple
            compare(logoPaths.currentColorway, "bluePurple", "Should fallback to bluePurple for invalid colorway")
        } finally {
            Settings.logoColorway = originalColorway
        }
    }
    
    function test_colorway_change_updates_window_icon() {
        // Skip if WindowShell not available (non-Windows platforms)
        if (typeof WindowShell === 'undefined') {
            skip("WindowShell not available in test environment")
        }
        
        // This is a manual test - verify that window icon updates
        // In automated tests, we can only verify the Settings change
        if (typeof Settings === 'undefined') {
            skip("Settings singleton not available in test environment")
        }
        
        const originalColorway = Settings.logoColorway
        
        try {
            // Change colorway multiple times
            const colorways = ["limeGreen", "orangeYellow", "babyPinkPurple", "bluePurple"]
            
            for (const colorway of colorways) {
                Settings.logoColorway = colorway
                wait(100)
                compare(logoPaths.currentColorway, colorway, `Colorway should be ${colorway}`)
            }
        } finally {
            Settings.logoColorway = originalColorway
        }
    }
}
