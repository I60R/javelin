use clap_complete::shells::{Zsh, Bash, Fish};
use clap::CommandFactory;

use std::{env, fs, error::Error, path::PathBuf};


#[allow(dead_code)]
mod javelin {
    include!("src/cli.rs");
}


fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = PathBuf::from(
        env::var("OUT_DIR")
        .unwrap()
    ).join("shell_completions");

    fs::create_dir_all(&out_dir)?;
    eprintln!("Shell completions would be generated in: {}", out_dir.display());

    let mut app = javelin::Args::command();
    clap_complete::generate_to(Zsh , &mut app, "javelin", &out_dir)?;
    clap_complete::generate_to(Bash, &mut app, "javelin", &out_dir)?;
    clap_complete::generate_to(Fish, &mut app, "javelin", &out_dir)?;

    Ok(())
}
