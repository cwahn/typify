use typify_impl::TypeSpace;

#[test]
fn names_change_without_reordering_variants_or_renaming_payload_types() {
    let schema = serde_json::from_str(include_str!("variant-names.json")).unwrap();
    let mut space = TypeSpace::default();
    space.add_root_schema(schema).unwrap();
    let file = syn::parse2::<syn::File>(space.to_stream()).unwrap();
    for (name, expected) in [
        ("FunctionCallOutput", vec!["Text", "Image", "File"]),
        (
            "OpaqueRecovery",
            vec!["ContainerAuto", "LocalEnvironment", "ContainerReference"],
        ),
        ("Recovery272", vec!["Dropbox", "Gmail", "Sharepoint"]),
        ("ApplyPatchOperation", vec!["Create", "Delete", "Update"]),
        ("PayloadPatchOperation", vec!["Create", "Delete"]),
        ("TaggedOperation", vec!["Create", "Delete"]),
        ("TransferState", vec!["Queued", "Running"]),
    ] {
        let variants = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == name => Some(
                    item.variants
                        .iter()
                        .map(|variant| variant.ident.to_string())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .unwrap();
        assert_eq!(variants, expected, "{name}");
    }
    for name in ["CreateFileOperation", "DeleteFileOperation"] {
        assert!(file
            .items
            .iter()
            .any(|item| matches!(item, syn::Item::Struct(item) if item.ident == name)));
    }
}

#[test]
fn repeated_registration_keeps_type_identity() {
    let schema = serde_json::from_value(serde_json::json!({
        "title": "ApplyPatchOperation",
        "type": "string",
        "enum": ["CreateFileOperation", "DeleteFileOperation"],
        "default": "DeleteFileOperation"
    }))
    .unwrap();
    let mut space = TypeSpace::default();
    let id = space.add_type(&schema).unwrap();
    assert_eq!(id, space.add_type(&schema).unwrap());
    assert_eq!(space.get_type(&id).unwrap().name(), "ApplyPatchOperation");
}
