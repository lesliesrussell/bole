// bole-6bd
use std::path::Path;
use crate::acl::Accessor;
use crate::error::Result;
use crate::repo::Repository;

pub async fn project_to_git(
    _repo: &Repository,
    _target_path: &Path,
    _accessor: &Accessor,
) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn stub_returns_ok() {
        let dir = tempdir().unwrap();
        let repo = Repository::memory();
        let accessor = Accessor::privileged();
        let result = project_to_git(&repo, dir.path(), &accessor).await;
        assert!(result.is_ok());
    }
}
