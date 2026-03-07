use crate::db::init_files_group;

/// Test Salsa database used by filesystem unit tests.
///
/// It is pre-initialized via [`init_files_group`] and can be cloned safely
/// across test helpers that need `#[salsa::db]` storage access.
#[salsa::db]
#[derive(Clone)]
pub struct FilesDatabaseForTesting {
    storage: salsa::Storage<FilesDatabaseForTesting>,
}

#[salsa::db]
impl salsa::Database for FilesDatabaseForTesting {}

impl Default for FilesDatabaseForTesting {
    fn default() -> Self {
        let mut res = Self {
            storage: Default::default(),
        };
        init_files_group(&mut res);
        res
    }
}
