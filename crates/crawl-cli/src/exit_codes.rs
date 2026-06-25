use crawl_core::vault_core::Error as VaultError;
use crawl_core::Error;

#[allow(dead_code)]
pub const SUCCESS: i32 = 0;
pub const GENERAL: i32 = 1;
pub const INVALID_USAGE: i32 = 2;
pub const NO_VAULT: i32 = 3;
pub const VAULT_CORRUPTION: i32 = 4;
pub const CONFIG_ERROR: i32 = 5;
pub const IO_ERROR: i32 = 6;
pub const LOCK_CONTENTION: i32 = 7;
/// A source could not be reached (mount missing, auth/network failure).
pub const UNREACHABLE: i32 = 10;

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
        Error::NoSuchSource(_)
        | Error::DuplicateSource(_)
        | Error::UnknownKind(_)
        | Error::UnknownStrategy(_) => INVALID_USAGE,
        Error::Unreachable { .. } | Error::SharePoint(_) => UNREACHABLE,
        _ => GENERAL,
    }
}
