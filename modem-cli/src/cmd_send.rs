use crate::Profile;
use std::path::PathBuf;
pub fn run(_profile: Profile, _input: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("`send` not implemented yet — use `tx-wav`");
}
