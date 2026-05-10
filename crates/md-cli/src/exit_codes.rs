use md_core::vault_core::Error as VaultError;
use md_core::Error;

#[allow(dead_code)]
pub const SUCCESS: i32 = 0;
pub const GENERAL: i32 = 1;
#[allow(dead_code)]
pub const INVALID_USAGE: i32 = 2;
pub const NO_VAULT: i32 = 3;
pub const VAULT_CORRUPTION: i32 = 4;
pub const CONFIG_ERROR: i32 = 5;
pub const IO_ERROR: i32 = 6;
pub const LOCK_CONTENTION: i32 = 7;
pub const CONFLICT: i32 = 9;

pub fn for_error(e: &Error) -> i32 {
    match e {
        Error::Vault(v) => match v {
            VaultError::NoState { .. } => NO_VAULT,
            VaultError::SchemaMismatch { .. } => VAULT_CORRUPTION,
            VaultError::LockContention => LOCK_CONTENTION,
            VaultError::Io(_) => IO_ERROR,
            _ => GENERAL,
        },
        Error::Config(_) => CONFIG_ERROR,
        Error::Io(_) => IO_ERROR,
        Error::Conflict { .. } => CONFLICT,
        _ => GENERAL,
    }
}
