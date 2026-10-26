// Copyright 2022 Oxide Computer Company

use quote::quote;
use schemars::{gen::SchemaGenerator, schema::RootSchema, schema::Schema, JsonSchema};
use serde::Serialize;
use typify_impl::{TypeSpace, TypeSpacePatch, TypeSpaceSettings};

#[allow(dead_code)]
#[derive(JsonSchema)]
struct CompoundType {
    value1: String,
    value2: u64,
}

#[allow(dead_code)]
#[derive(JsonSchema, Serialize)]
enum StringEnum {
    One,
    Two,
    BuckleMyShoe,
}

#[allow(dead_code)]
#[derive(JsonSchema, Serialize)]
#[serde(default = "default_pair")]
struct Pair {
    a: StringEnum,
    b: StringEnum,
}

fn default_pair() -> Pair {
    Pair {
        a: StringEnum::One,
        b: StringEnum::Two,
    }
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct AllTheTraits {
    ok: String,
}

fn add_type<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = T::json_schema(generator).into_object();
    schema.metadata().title = Some(T::schema_name());
    schema.into()
}

#[test]
fn test_generation() {
    let mut type_space = TypeSpace::new(
        TypeSpaceSettings::default()
            .with_derive("JsonSchema".to_string())
            .with_type_mod("types")
            .with_struct_builder(true)
            .with_patch(
                "AllTheTraits",
                TypeSpacePatch::default()
                    .with_derive("Hash")
                    .with_derive("Ord")
                    .with_derive("PartialOrd")
                    .with_derive("Eq")
                    .with_derive("PartialEq"),
            ),
    );

    let mut generator = SchemaGenerator::default();
    let body_schema = add_type::<CompoundType>(&mut generator);
    let string_schema = add_type::<String>(&mut generator);
    let opt_int_schema = add_type::<Option<u32>>(&mut generator);
    let strenum_schema = add_type::<StringEnum>(&mut generator);
    let pair_schema = add_type::<Pair>(&mut generator);
    let all_the_traits = add_type::<AllTheTraits>(&mut generator);

    type_space
        .add_ref_types(generator.take_definitions())
        .unwrap();

    let tid = type_space.add_type(&body_schema).unwrap();
    let t = type_space.get_type(&tid).unwrap();
    let ret = t.ident();
    let body = t.parameter_ident();

    let string_id = type_space.add_type(&string_schema).unwrap();
    let string = type_space.get_type(&string_id).unwrap().parameter_ident();
    let opt_int_id = type_space.add_type(&opt_int_schema).unwrap();
    let opt_int = type_space.get_type(&opt_int_id).unwrap().parameter_ident();
    let strenum_id = type_space.add_type(&strenum_schema).unwrap();
    let strenum = type_space.get_type(&strenum_id).unwrap().parameter_ident();
    let _ = type_space.add_type(&pair_schema).unwrap();
    let _ = type_space.add_type(&all_the_traits).unwrap();

    let types = type_space.to_stream();

    let file = quote! {
        mod types {
            #types
        }

        pub fn do_stuff(
            body: #body,
            string: #string,
            opt_int: #opt_int,
            strenum: #strenum,
        ) -> #ret {
            todo!()
        }
    };

    let fmt = rustfmt_wrapper::rustfmt(file.to_string()).unwrap();

    expectorate::assert_contents("tests/generator.out", fmt.as_str());
}

#[test]
fn test_nullable_names_and_registration() {
    let schema: RootSchema = serde_json::from_str(include_str!("nullable-names.json")).unwrap();
    let mut type_space = TypeSpace::default();
    type_space.add_root_schema(schema).unwrap();
    let output = rustfmt_wrapper::rustfmt(type_space.to_stream().to_string()).unwrap();
    for name in [
        "NullableObject",
        "OneOfEffort",
        "ReasoningEffort",
        "TitledNullableEnum",
    ] {
        assert!(
            output.contains(&format!(
                "pub struct {name}(pub ::std::option::Option<{name}Inner>);"
            )),
            "{output}"
        );
    }
    assert!(
        output.contains("pub struct RefNullable(pub ::std::option::Option<AuthorAssociation>);")
    );
    let file: syn::File = syn::parse_str(&output).unwrap();
    let mut names = std::collections::BTreeSet::new();
    for item in file.items {
        let name = match item {
            syn::Item::Struct(item) => item.ident,
            syn::Item::Enum(item) => item.ident,
            _ => continue,
        };
        assert!(
            names.insert(name.to_string()),
            "duplicate emitted type {name}"
        );
    }
}

#[test]
fn test_inline_title_registration_preserves_distinct_schemas() {
    let mut type_space = TypeSpace::default();
    let first: Schema = serde_json::from_value(serde_json::json!({
        "title": "Shared", "type": "string", "enum": ["first"]
    }))
    .unwrap();
    let second: Schema = serde_json::from_value(serde_json::json!({
        "title": "Shared", "type": "string", "enum": ["second"]
    }))
    .unwrap();
    let first_id = type_space.add_type(&first).unwrap();
    assert_eq!(first_id, type_space.add_type(&first).unwrap());
    let second_id = type_space.add_type(&second).unwrap();
    assert_ne!(first_id, second_id);
    assert_eq!(type_space.get_type(&first_id).unwrap().name(), "Shared");
    assert_eq!(type_space.get_type(&second_id).unwrap().name(), "Shared2");
}

#[test]
fn test_concrete_defaults_and_self_constructors() {
    let schema: RootSchema = serde_json::from_str(include_str!("concrete-defaults.json")).unwrap();
    let mut type_space = TypeSpace::new(TypeSpaceSettings::default().with_struct_builder(true));
    type_space.add_root_schema(schema).unwrap();
    let output = rustfmt_wrapper::rustfmt(type_space.to_stream().to_string()).unwrap();
    assert!(!output.contains("Default::default()"), "{output}");
    assert!(
        output.contains("items: <::std::vec::Vec<Nested>>::default()"),
        "{output}"
    );
    assert!(
        output.contains("items: Ok(<::std::vec::Vec<super::Nested>>::default())"),
        "{output}"
    );
    assert!(output.contains("Self::Second"), "{output}");
    assert!(
        output.contains("<builder::Model as ::std::default::Default>::default()"),
        "{output}"
    );
    assert!(
        output.contains("value.chars().count() > 10_485_760usize"),
        "{output}"
    );
    assert!(
        output.contains("value.chars().count() < 10_000usize"),
        "{output}"
    );
}

#[test]
fn test_enum_default_derivation_is_limited_to_selected_unit_variants() {
    let schema: RootSchema = serde_json::from_str(include_str!("concrete-defaults.json")).unwrap();
    let mut type_space = TypeSpace::default();
    type_space.add_root_schema(schema).unwrap();
    let stream = type_space.to_stream();
    let file = syn::parse2::<syn::File>(stream.clone()).unwrap();
    for (name, expected) in [
        ("Choice", vec!["Second"]),
        ("RenamedChoice", vec!["InProgress"]),
        ("Tagged", vec!["Empty"]),
        ("Payload", vec![]),
        ("TaggedPayload", vec![]),
        ("NoDefault", vec![]),
    ] {
        let item = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == name => Some(item),
                _ => None,
            })
            .unwrap();
        let marked = item
            .variants
            .iter()
            .filter(|variant| {
                variant
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("default"))
            })
            .map(|variant| variant.ident.to_string())
            .collect::<Vec<_>>();
        assert_eq!(marked, expected, "{name}");
    }
    let output = rustfmt_wrapper::rustfmt(stream.to_string()).unwrap();
    assert!(
        output.contains("pub(super) const fn helpers_unit()"),
        "{output}"
    );
    assert!(
        output.contains("pub(super) fn helpers_payload()"),
        "{output}"
    );
    assert!(
        output.contains("impl ::std::default::Default for Payload"),
        "{output}"
    );
    assert!(
        output.contains("impl ::std::default::Default for TaggedPayload"),
        "{output}"
    );
    for name in ["Choice", "RenamedChoice", "Tagged", "NoDefault"] {
        assert!(
            !output.contains(&format!("impl ::std::default::Default for {name} {{")),
            "{output}"
        );
    }
}

#[test]
fn test_model_and_property_docs_render_without_rewriting_schema_examples() {
    fn doc_text(attributes: &[syn::Attribute]) -> String {
        attributes
            .iter()
            .filter_map(|attribute| {
                if !attribute.path().is_ident("doc") {
                    return None;
                }
                let syn::Meta::NameValue(meta) = &attribute.meta else {
                    return None;
                };
                let syn::Expr::Lit(value) = &meta.value else {
                    return None;
                };
                let syn::Lit::Str(value) = &value.lit else {
                    return None;
                };
                Some(value.value())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    let schema: RootSchema = serde_json::from_str(include_str!("concrete-defaults.json")).unwrap();
    let mut type_space = TypeSpace::default();
    type_space.add_root_schema(schema).unwrap();
    let file = syn::parse2::<syn::File>(type_space.to_stream()).unwrap();
    let model = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "Model" => Some(item),
            _ => None,
        })
        .unwrap();
    let model_doc = doc_text(&model.attrs);
    assert!(
        model_doc.contains("Use `OpenAI` with `field_name`."),
        "{model_doc}"
    );
    assert!(
        model_doc.contains("```json\n{\"field_name\": \"OpenAI\"}\n```"),
        "{model_doc}"
    );
    let optional = model
        .fields
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|name| name == "optional"))
        .unwrap();
    assert_eq!(
        doc_text(&optional.attrs),
        "Returned `field_name` from `OpenAI`."
    );
}
