use lynx_compose::env_file::load_env_files;
use lynx_compose::ComposeError;
use std::io::Write;

#[test]
fn basic_key_value() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join("app.env")).unwrap();
    writeln!(f, "# comment").unwrap();
    writeln!(f).unwrap();
    writeln!(f, "DB_HOST=localhost").unwrap();
    writeln!(f, "PORT=5432").unwrap();
    writeln!(f, "NOVALUE").unwrap();

    let map = load_env_files(&["app.env".to_string()], dir.path()).unwrap();
    assert_eq!(map["DB_HOST"], "localhost");
    assert_eq!(map["PORT"], "5432");
    assert_eq!(map["NOVALUE"], "");
}

#[test]
fn string_or_list_single() {
    use lynx_compose::compose::types::StringOrList;
    assert_eq!(
        StringOrList::Single("file.env".to_string()).to_list(),
        vec!["file.env"]
    );
}

#[test]
fn string_or_list_many() {
    use lynx_compose::compose::types::StringOrList;
    let sol = StringOrList::List(vec!["a.env".to_string(), "b.env".to_string()]);
    assert_eq!(sol.to_list().len(), 2);
}

#[test]
fn first_file_wins_for_duplicate_keys() {
    let dir = tempfile::tempdir().unwrap();

    let mut a = std::fs::File::create(dir.path().join("a.env")).unwrap();
    writeln!(a, "KEY=from_a").unwrap();

    let mut b = std::fs::File::create(dir.path().join("b.env")).unwrap();
    writeln!(b, "KEY=from_b").unwrap();

    let map = load_env_files(&["a.env".to_string(), "b.env".to_string()], dir.path()).unwrap();
    assert_eq!(map["KEY"], "from_a");
}

#[test]
fn quoted_values_preserved_as_is() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join("q.env")).unwrap();
    writeln!(f, r#"KEY="value with spaces""#).unwrap();

    let map = load_env_files(&["q.env".to_string()], dir.path()).unwrap();
    // Per compose-spec, quotes are preserved literally.
    assert_eq!(map["KEY"], "\"value with spaces\"");
}

#[test]
fn value_with_equals_sign() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join("eq.env")).unwrap();
    writeln!(f, "KEY=val=ue=more").unwrap();

    let map = load_env_files(&["eq.env".to_string()], dir.path()).unwrap();
    assert_eq!(map["KEY"], "val=ue=more");
}

#[test]
fn missing_env_file_errors() {
    let dir = tempfile::tempdir().unwrap();
    let result = load_env_files(&["does_not_exist.env".to_string()], dir.path());
    assert!(matches!(result, Err(ComposeError::FileNotFound(_))));
}
