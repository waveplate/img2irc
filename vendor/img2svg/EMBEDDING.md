Source: the published img2svg 0.1.15 crate, https://crates.io/crates/img2svg/0.1.15
Upstream: https://github.com/yingkitw/img2svg
License: Apache-2.0 (see LICENSE).

The library sources are unchanged except for a `quiet` feature that suppresses
plain stage messages in enhanced_vectorizer::stage_log. Without this patch,
upstream writes to stderr even with progress=false, corrupting img2irc's TUI.
Progress-bar mode remains available to other callers. The manifest includes
only the library target and its dependencies; upstream CLI and MCP binaries
are not built by img2irc.
