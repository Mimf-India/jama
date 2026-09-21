use clap::CommandFactory;
use clap_complete::{generate, Shell};

use crate::{Cli, CliError};

pub fn run(shell: Shell) -> Result<(), CliError> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut std::io::stdout());
    Ok(())
}
