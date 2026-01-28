# Changelog

## [0.1.1] - 2026-01-28

### Fixed
- **Critical Performance**: Fixed O(N) payload complexity for single-record updates in Streaming views. Previously, any update triggered a full view refresh (sending all records as "Updated"). Now, strictly sends only the modified records.
- **Latency**: Reduced single-record update latency from ~1.4ms to ~8µs for 10k record views (observed in benchmarks).
- **Bandwidth**: Massive reduction in update payload size for Streaming views.

### Changed
- Refactored `View::process_batch` to perform result filtering based on `ViewResultFormat`.
- Optimized `View::build_single_update` to align with new batch processing logic.

### Added
- Comprehensive test suite `tests/update_fix_test.rs` covering streaming behavior, subqueries, and mixed operations.
- New benchmark `benchmark_streaming_vs_flat_single_update` proving the performance gains.
