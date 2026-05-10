pub mod add;
pub mod config;
pub mod convert;
pub mod info;
pub mod init;
pub mod ls;
pub mod prune;
pub mod rm;
pub mod show;
pub mod status;
pub mod whence;

use md_core::MdVault;
use std::path::Path;

pub fn open_vault(vault_arg: Option<&Path>) -> anyhow::Result<MdVault> {
    let v = match vault_arg {
        Some(p) => MdVault::open(p)?,
        None => {
            let cwd = std::env::current_dir()?;
            MdVault::discover(&cwd)?
        }
    };
    Ok(v)
}
