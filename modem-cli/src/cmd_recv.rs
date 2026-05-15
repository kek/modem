use crate::Profile;
use std::path::PathBuf;
pub fn run(_profile: Profile, _output: Option<PathBuf>, _hex: bool) -> anyhow::Result<()> {
    anyhow::bail!("`recv` not implemented yet — use `rx-wav`");
}
