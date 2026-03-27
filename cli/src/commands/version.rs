use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run() -> Result<()> {
    println!("{}", format!("inai v{}", env!("INAI_VERSION")).bold());
    println!(
        "  {:20} {}",
        "Build date:".bright_black(),
        env!("INAI_BUILD_DATE")
    );
    println!(
        "  {:20} {}",
        "Platform:".bright_black(),
        std::env::consts::OS
    );
    println!("  {:20} {}", "Arch:".bright_black(), std::env::consts::ARCH);
    println!(
        "  {:20} {}",
        "Rust compiler:".bright_black(),
        env!("INAI_RUSTC_VERSION").bright_black()
    );
    Ok(())
}
