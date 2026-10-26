use typify_impl::{TypeSpace, TypeSpaceSettings};

#[test]
fn regex_item_precedes_statements_and_map_storage_is_unchanged() {
    let schema = serde_json::from_str(include_str!("concrete-defaults.json")).unwrap();
    let mut space = TypeSpace::default();
    space.add_root_schema(schema).unwrap();
    let file = syn::parse2::<syn::File>(space.to_stream()).unwrap();
    let bounded_from_str = file.items.iter().find_map(|item| {
        let syn::Item::Impl(item) = item else { return None; };
        if !matches!(item.self_ty.as_ref(), syn::Type::Path(path) if path.path.is_ident("Bounded")) {
            return None;
        }
        item.items.iter().find_map(|item| match item {
            syn::ImplItem::Fn(function) if function.sig.ident == "from_str" => Some(function),
            _ => None,
        })
    }).unwrap();
    assert!(matches!(
        bounded_from_str.block.stmts.first(),
        Some(syn::Stmt::Item(syn::Item::Static(_)))
    ));
    for name in ["StringMap", "OptionalStringMap"] {
        let model = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == name => Some(item),
                _ => None,
            })
            .unwrap();
        assert!(model.generics.params.is_empty());
        assert!(!quote::quote!(#model)
            .to_string()
            .contains("__TypifyBuildHasher"));
    }
}

#[test]
fn custom_map_choice_does_not_gain_hashmap_conversion_parameters() {
    let schema = serde_json::from_str(include_str!("concrete-defaults.json")).unwrap();
    let mut settings = TypeSpaceSettings::default();
    settings.with_map_type("::std::collections::BTreeMap");
    let mut space = TypeSpace::new(&settings);
    space.add_root_schema(schema).unwrap();
    let output = space.to_stream().to_string();
    assert!(output.contains("BTreeMap"));
    assert!(!output.contains("__TypifyBuildHasher"));
}
