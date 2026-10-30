//! Compatibility entry point. Video rendering and the editor live in img2irc.
fn main() -> std::process::ExitCode {
    img2irc_rs::video::run(std::env::args_os())
}
