use sem_lang_memory::definitions;
use sem_lang_repository::{HeadStore, Repository};
use sem_lang_store_memory::InMemoryRepository;

#[test]
fn adoption_replay_after_later_head_returns_original_reference_and_rejects_before_allocation() {
    let mut repo = InMemoryRepository::new();
    let initial = repo.head_revision("definitions-owner").unwrap();
    let first =
        definitions::adopt(&mut repo, "owner", None, &[], "decision:first", &initial).unwrap();
    let revision = repo.head_revision("definitions-owner").unwrap();
    let second = definitions::adopt(
        &mut repo,
        "owner",
        Some(&first),
        &[],
        "decision:second",
        &revision,
    )
    .unwrap();
    assert_ne!(first, second);
    let head = repo.head_revision("definitions-owner").unwrap();
    let before = repo.verify().unwrap();
    for _ in 0..100 {
        assert_eq!(
            definitions::adopt(&mut repo, "owner", None, &[], "decision:first", &initial).unwrap(),
            first
        );
        assert_eq!(
            definitions::adopt(
                &mut repo,
                "owner",
                Some(&first),
                &[],
                "decision:second",
                &revision
            )
            .unwrap(),
            second
        );
        assert!(
            definitions::adopt(&mut repo, "owner", None, &[], "decision:stale", &initial).is_err()
        );
        assert!(
            definitions::adopt(
                &mut repo,
                "owner",
                Some(&second),
                &[],
                "decision:first",
                &head
            )
            .is_err()
        );
    }
    assert_eq!(repo.load_head("definitions-owner").unwrap(), Some(second));
    assert_eq!(repo.head_revision("definitions-owner").unwrap(), head);
    let after = repo.verify().unwrap();
    assert_eq!(after.objects_checked, before.objects_checked);
    assert_eq!(after.revisions_checked, before.revisions_checked);
    assert!(after.errors.is_empty());
}
