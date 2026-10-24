use super::*;
#[test]
fn undeclared_dependency_and_conflicting_producer_definitions_never_adopt() {
    assert!(
        VocabularyModule::from_sem(&SOURCE.replace(
            "Dependencies: sem-lang/standard/semantic",
            "Dependencies: none"
        ))
        .is_err()
    );
    let mut selected = modules();
    selected.push(selected.last().unwrap().clone());
    let temp = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut repo = FsRepository::create(temp.path().join("source-store")).unwrap();
    assert!(
        definitions::adopt(
            &mut repo,
            NAMESPACE,
            None,
            &selected,
            "test:conflict",
            &RevisionRef::default()
        )
        .is_err()
    );
    assert!(definitions::read(&repo, NAMESPACE).unwrap().is_none());
}
