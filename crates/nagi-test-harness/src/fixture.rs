use crate::{FakeFilesystem, Result, TemporaryRoot};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureIds {
    pub user_id: String,
    pub profile_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureCleanupReport {
    pub root_identity: u64,
    pub files_removed: usize,
    pub root_removed: bool,
}

pub struct UserProfileFixture {
    ids: FixtureIds,
    root: TemporaryRoot,
}

impl UserProfileFixture {
    pub fn new(filesystem: &FakeFilesystem, sequence: u64) -> Result<Self> {
        let root = filesystem.temporary_root()?;
        let prefix = format!("fixture-{sequence:016x}");
        Ok(Self {
            ids: FixtureIds {
                user_id: format!("{prefix}-user"),
                profile_id: format!("{prefix}-profile"),
                session_id: format!("{prefix}-session"),
            },
            root,
        })
    }

    pub fn ids(&self) -> &FixtureIds {
        &self.ids
    }

    pub fn root(&self) -> TemporaryRoot {
        self.root.clone()
    }

    pub fn teardown(self) -> FixtureCleanupReport {
        let root_removed = self.root.handle_count() == 1;
        let report = FixtureCleanupReport {
            root_identity: self.root.root_identity(),
            files_removed: self.root.file_count(),
            root_removed,
        };
        drop(self);
        report
    }
}
