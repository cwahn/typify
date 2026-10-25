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
