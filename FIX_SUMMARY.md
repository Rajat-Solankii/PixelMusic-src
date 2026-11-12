# Fix for Intermittent Blank Album Covers

## Summary

This fix addresses the issue where cover images sometimes fail silently and stay blank, implementing proper error handling, retry logic, and URL normalization.

## Changes Made

### 1. **URL Normalization (crates/sf-services/src/youtube/parsers.rs)**

Added comprehensive URL normalization for YouTube Music / Google thumbnail URLs:

- **Protocol-relative URLs**: Automatically convert `//lh3.googleusercontent.com/...` to `https://lh3.googleusercontent.com/...`
- **Malformed size parameters**: Fix `=w300-h300` and `=s300` parameters to proper format
- **YouTube image URLs**: Added support for `ytimg.com` URLs
- **Size clamping**: Ensure pixel sizes are within reasonable bounds (48-1920px)
- **Missing size params**: Append default size parameters when missing

#### Modified Functions:
- `normalize_thumbnail_url()`: New function to handle URL normalization
- `sized()`: Enhanced to handle protocol-relative URLs and ytimg.com URLs
- `thumbnail_url()`: Updated to use new normalization

### 2. **Prevent Caching of Failed Images (crates/materialfy/cpp/mf_shell.cpp)**

Added `NoCacheReply` wrapper class that:
- Monitors network replies for errors
- Automatically removes failed downloads from cache
- Prevents permanent caching of failed/errored images
- Handles various error types (timeout, connection refused, etc.)

**Key Features**:
- Transparent wrapper around QNetworkReply
- Forwards all signals and data access methods
- Automatically removes URL from disk cache on error
- Maintains existing Spotify mirror failover logic

### 3. **Exponential Backoff Retry Logic (src/materialfy/qml/Md3/MorphImage.qml)**

Enhanced retry mechanism with proper exponential backoff:

- **Retry Schedule**: 1.5s → 3s → 6s (max 3 retries)
- **Exponential backoff**: Each retry doubles the delay
- **Capped delay**: Maximum wait time of 6 seconds
- **Automatic reset**: Timer resets to default after each retry

**Previous behavior**: Single retry after 1.5s
**New behavior**: 3 retries with exponential backoff (1.5s, 3s, 6s)

### 4. **Bug Fixes**

#### crates/sf-services/src/youtube/duration_cache.rs
- Fixed borrow checker error when evicting old entries
- Added `drop()` to release immutable borrow before mutation

#### crates/mf-platform/src/settings.rs
- Added missing `LogoColorway` enum variant to match statements
- Fixed non-exhaustive pattern errors

## Testing

Added comprehensive tests for URL normalization:
- Protocol-relative URL conversion
- ytimg.com URL handling
- Size parameter replacement
- Size clamping (min/max bounds)
- Various size parameter formats (=w, =s)

All tests pass successfully.

## Files Modified

1. `crates/sf-services/src/youtube/parsers.rs`
2. `crates/materialfy/cpp/mf_shell.cpp`
3. `src/materialfy/qml/Md3/MorphImage.qml`
4. `crates/sf-services/src/youtube/duration_cache.rs`
5. `crates/mf-platform/src/settings.rs`
6. `crates/sf-services/src/youtube/parser_tests.rs` (tests)

## Benefits

1. **More resilient image loading**: Failed downloads are automatically retried with backoff
2. **No permanent blank images**: Failed images are not cached, allowing future load attempts
3. **Better URL handling**: Handles various malformed YouTube/Google thumbnail URLs
4. **Improved user experience**: Less likely to see blank album covers
5. **Already has concurrency limiting**: `ImageGate` singleton already limits concurrent thumbnail requests to 12

## Backward Compatibility

All changes are backward compatible:
- URL normalization handles all existing URL formats
- Retry logic is more aggressive but doesn't change the fallback mechanism
- Cache invalidation only affects failed downloads, not successful ones
- No API changes required
