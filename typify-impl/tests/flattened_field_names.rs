use std::{fs, process::Command, time::SystemTime};

use schemars::schema::RootSchema;
use typify_impl::{TypeSpace, TypeSpaceSettings};

fn schema() -> RootSchema {
    serde_json::from_str(include_str!("flattened-field-names/schema.json")).unwrap()
}

fn generated() -> syn::File {
    generate(schema(), true)
}

fn builder_api_schema() -> RootSchema {
    serde_json::from_str(include_str!("flattened-field-names/builder-api.json")).unwrap()
}

fn generate(schema: RootSchema, builders: bool) -> syn::File {
    let mut settings = TypeSpaceSettings::default();
    settings.with_struct_builder(builders);
    let mut space = TypeSpace::new(&settings);
    space.add_root_schema(schema).unwrap();
    syn::parse2(space.to_stream()).unwrap()
}

fn model<'a>(items: &'a [syn::Item], name: &str) -> &'a syn::ItemStruct {
    items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == name => Some(item),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing struct {name}"))
}

fn field_names(item: &syn::ItemStruct) -> Vec<String> {
    item.fields
        .iter()
        .map(|field| field.ident.as_ref().unwrap().to_string())
        .collect()
}

#[test]
fn approved_names_reach_models_builders_and_setters_without_wire_renames() {
    let file = generated();
    let builder = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Mod(item) if item.ident == "builder" => {
                Some(&item.content.as_ref().unwrap().1)
            }
            _ => None,
        })
        .unwrap();
    for (name, expected) in [
        (
            "ActualModelIdsCompaction",
            vec!["model_ids_responses", "string", "null"],
        ),
        ("ObjectAny", vec!["request_id", "response_body"]),
        (
            "CollisionFields",
            vec![
                "string",
                "string_3",
                "string_2",
                "type_",
                "http_response",
                "subtype_5",
                "subtype_6",
                "subtype_5_2",
                "escaped_name",
                "self_",
                "type_2",
            ],
        ),
    ] {
        let item = model(&file.items, name);
        assert_eq!(field_names(item), expected, "{name}");
        assert_eq!(
            field_names(model(builder, name)),
            expected,
            "builder::{name}"
        );
        let setters = builder.iter().find_map(|item| {
            let syn::Item::Impl(item) = item else { return None; };
            if item.trait_.is_some()
                || !matches!(item.self_ty.as_ref(), syn::Type::Path(path) if path.path.is_ident(name))
            {
                return None;
            }
            Some(item.items.iter().filter_map(|item| match item {
                syn::ImplItem::Fn(item) => Some(item.sig.ident.to_string()),
                _ => None,
            }).collect::<Vec<_>>())
        }).unwrap();
        assert_eq!(setters, expected, "builder::{name} setters");
        for field in &item.fields {
            // Inspect the actual attribute tokens, not the Rust field spelling.
            let attrs = field
                .attrs
                .iter()
                .map(|attr| quote::quote!(#attr).to_string())
                .collect::<String>();
            assert!(attrs.contains("flatten"));
            assert!(attrs.contains("::std::option::Option::is_none"));
            assert!(!attrs.contains("rename"));
            assert!(!attrs.contains("default"));
        }
    }
    let fields = model(&file.items, "ActualModelIdsCompaction")
        .fields
        .iter()
        .map(|field| {
            let ty = &field.ty;
            quote::quote!(#ty).to_string().replace(' ', "")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        [
            "::std::option::Option<ModelIdsResponses>",
            "::std::option::Option<::std::string::String>",
            "::std::option::Option<()>",
        ]
    );
    // allOf still merges real JSON properties rather than introducing fields.
    assert_eq!(
        field_names(model(&file.items, "ObjectAll")),
        ["enabled", "request_id"]
    );
    assert_eq!(
        field_names(model(&file.items, "FlattenedFieldNames")),
        ["compaction"]
    );
    assert!(
        file.items.iter().any(|item| {
            matches!(item, syn::Item::Struct(item) if item.ident == "CollisionFieldsSubtype5")
        }),
        "index-based branch type suggestions must remain unchanged"
    );
}

#[test]
fn names_are_stable_across_forward_and_reverse_reference_registration() {
    let definitions = schema().definitions.into_iter().collect::<Vec<_>>();
    let mut forward = TypeSpace::default();
    forward.add_ref_types(definitions.clone()).unwrap();
    let mut reverse = TypeSpace::default();
    reverse
        .add_ref_types(definitions.into_iter().rev())
        .unwrap();
    let forward: syn::File = syn::parse2(forward.to_stream()).unwrap();
    let reverse: syn::File = syn::parse2(reverse.to_stream()).unwrap();
    for name in ["ActualModelIdsCompaction", "ObjectAny", "CollisionFields"] {
        assert_eq!(
            field_names(model(&forward.items, name)),
            field_names(model(&reverse.items, name))
        );
    }
    assert_eq!(
        quote::quote!(#forward).to_string(),
        quote::quote!(#reverse).to_string()
    );
}

#[test]
fn builder_method_names_are_reserved_in_both_builder_modes() {
    let enabled = generate(builder_api_schema(), true);
    let disabled = generate(builder_api_schema(), false);
    let builder_items = enabled
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Mod(item) if item.ident == "builder" => {
                Some(&item.content.as_ref().unwrap().1)
            }
            _ => None,
        })
        .unwrap();
    for (name, expected) in [
        ("DefaultFactory", vec!["default_value", "string"]),
        ("TryIntoFactory", vec!["try_into_value", "string"]),
        (
            "BuilderApiFields",
            vec![
                "default_value",
                "default_value_3",
                "default_value_2",
                "clone_value",
                "clone_from_value",
                "from_value",
                "try_from_value",
                "into_value",
                "try_into_value",
                "fmt_value",
                "to_owned_value",
                "clone_into_value",
                "borrow_value",
                "borrow_mut_value",
                "type_id_value",
                "string",
            ],
        ),
    ] {
        assert_eq!(field_names(model(&enabled.items, name)), expected, "{name}");
        assert_eq!(
            field_names(model(&disabled.items, name)),
            expected,
            "{name}, builders disabled"
        );
        assert_eq!(
            field_names(model(builder_items, name)),
            expected,
            "builder::{name}"
        );
        let enabled_fields = &model(&enabled.items, name).fields;
        let disabled_fields = &model(&disabled.items, name).fields;
        assert_eq!(
            quote::quote!(#enabled_fields).to_string(),
            quote::quote!(#disabled_fields).to_string()
        );
    }

    // These reservations are only for synthetic fields. Do not broaden this
    // fix into the existing, unrelated handling of actual JSON property names.
    for builders in [false, true] {
        let file = generate(
            serde_json::from_value(serde_json::json!({
                "title": "OrdinaryProperties",
                "type": "object",
                "properties": {
                    "clone": {"type": "string"}, "default": {"type": "string"},
                    "from": {"type": "string"}, "try_into": {"type": "string"}
                }
            }))
            .unwrap(),
            builders,
        );
        assert_eq!(
            field_names(model(&file.items, "OrdinaryProperties")),
            ["clone", "default", "from", "try_into"]
        );
    }
}

#[test]
fn generated_builders_compile_and_serde_matches_the_old_field_names() {
    // A separate, disposable consumer crate compiles the generated Rust, without
    // changing any workspace manifest or relying on pre-existing golden files.
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join("opencode").join(format!(
        "typify-flattened-fields-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        r#"
[package]
name = "typify-flattened-field-consumer"
version = "0.0.0"
edition = "2021"

[workspace]

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
"#,
    )
    .unwrap();
    // Reuse this checkout's dependency versions, including Serde, rather than
    // testing against whichever newer versions happen to be cached locally.
    fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.lock"),
        directory.join("Cargo.lock"),
    )
    .unwrap();
    let file = generated();
    let builder_api = generate(builder_api_schema(), true);
    let source = format!(
        "#[deny(clippy::struct_field_names)]\npub mod after {{ {} }}\n\
         #[deny(clippy::struct_field_names)]\npub mod builder_api {{ {} }}\n{}",
        quote::quote!(#file),
        quote::quote!(#builder_api),
        include_str!("flattened-field-names/behavior.rs"),
    );
    fs::write(directory.join("src/lib.rs"), source).unwrap();
    for arguments in [
        vec!["test", "--offline"],
        vec!["clippy", "--offline", "--tests"],
    ] {
        let mut command = Command::new(env!("CARGO"));
        if arguments[0] == "test" {
            command
                .env_remove("CARGO_ENCODED_RUSTFLAGS")
                .env("RUSTFLAGS", "-Dwarnings");
        }
        let output = command
            .args(arguments)
            .env_remove("CARGO_TARGET_DIR")
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "consumer retained at {}\n{}\n{}",
            directory.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_dir_all(directory).unwrap();
}
