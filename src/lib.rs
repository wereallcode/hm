#[cfg(all(
    feature = "cli",
    not(all(target_os = "macos", target_arch = "aarch64"))
))]
compile_error!("hm supports Apple Silicon macOS only");

#[cfg(feature = "cli")]
pub mod authorization;
pub mod hosts;
pub mod mutations;
pub mod output;
pub mod storage;
