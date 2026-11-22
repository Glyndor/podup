//! Sorted manifests include empty directories and hash every regular file.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
pub enum Content {
	Directory,
	File([u8; 32]),
	Symlink(PathBuf),
}

pub type Manifest = BTreeMap<PathBuf, (u32, Content)>;

pub fn snapshot(root: &Path) -> Manifest {
	fn visit(root: &Path, path: &Path, entries: &mut Manifest) {
		let metadata = fs::symlink_metadata(path).expect("stat snapshot entry");
		let content = if metadata.is_dir() {
			Content::Directory
		} else if metadata.is_file() {
			let mut file = fs::File::open(path).expect("open snapshot file");
			let mut hash = Sha256::new();
			let mut buf = [0u8; 8192];
			loop {
				let n = file.read(&mut buf).expect("hash snapshot file");
				if n == 0 {
					break;
				}
				hash.update(&buf[..n]);
			}
			Content::File(hash.finalize().into())
		} else if metadata.file_type().is_symlink() {
			Content::Symlink(fs::read_link(path).expect("snapshot symlink target"))
		} else {
			panic!("unexpected special file in snapshot: {}", path.display());
		};
		entries.insert(
			path.strip_prefix(root).unwrap().to_path_buf(),
			(metadata.permissions().mode(), content),
		);
		if metadata.is_dir() {
			for entry in fs::read_dir(path).expect("list snapshot directory") {
				visit(root, &entry.expect("snapshot entry").path(), entries);
			}
		}
	}
	let mut entries = Manifest::new();
	visit(root, root, &mut entries);
	entries
}
