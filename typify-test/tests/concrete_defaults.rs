#![deny(
    clippy::default_trait_access,
    clippy::use_self,
    clippy::derivable_impls,
    clippy::missing_const_for_fn,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::same_functions_in_if_condition,
    clippy::implicit_hasher,
    clippy::unreadable_literal
)]

include!(concat!(env!("OUT_DIR"), "/concrete_defaults.rs"));

#[test]
fn enum_defaults_keep_the_selected_unit_or_payload() {
    assert_eq!(serde_json::to_value(Choice::default()).unwrap(), "second");
    assert_eq!(
        serde_json::to_value(RenamedChoice::default()).unwrap(),
        "in-progress"
    );
    assert_eq!(serde_json::to_value(Payload::default()).unwrap(), 7);
    assert_eq!(
        serde_json::to_value(Tagged::default()).unwrap(),
        serde_json::json!({"kind": "empty"})
    );
    assert_eq!(
        serde_json::to_value(TaggedPayload::default()).unwrap(),
        serde_json::json!({"kind": "full", "value": "body", "count": 2})
    );
    let helpers = Helpers::default();
    assert_eq!(serde_json::to_value(helpers.unit).unwrap(), "second");
    assert_eq!(serde_json::to_value(helpers.payload).unwrap(), "text");
}

#[test]
fn concrete_defaults_preserve_nested_values_and_builder_types() {
    let model: Model = serde_json::from_value(serde_json::json!({
        "choice": "second", "nested": { "choice": "first" }, "label": "custom"
    }))
    .unwrap();
    assert_eq!(model.choice.to_string(), "second");
    assert_eq!(model.nested.choice.to_string(), "first");
    assert_eq!(model.label.to_string(), "custom");
    assert!(model.optional.is_none());
    assert!(model.items.is_empty());
    assert!(model.mapping.is_empty());

    let builder: builder::Model = Model::builder();
    assert!(Model::try_from(builder).is_err());
    let nested = Nested::try_from(Nested::builder().choice(Choice::Second)).unwrap();
    let built = Model::try_from(
        Model::builder()
            .choice(Choice::default())
            .nested(nested)
            .label("label".to_string()),
    )
    .unwrap();
    assert_eq!(built.choice.to_string(), "second");
    assert_eq!(built.nested.choice.to_string(), "second");
    assert_eq!(built.label.to_string(), "label");

    let mixed = Mixed::default();
    let built = Mixed::try_from(Mixed::builder()).unwrap();
    assert_eq!(mixed.count, 7);
    assert_eq!(built.count, mixed.count);
    assert_eq!(
        serde_json::to_value(&mixed).unwrap(),
        serde_json::to_value(&built).unwrap()
    );

    let roundtrip: Model = serde_json::from_value(serde_json::to_value(&model).unwrap()).unwrap();
    assert_eq!(roundtrip.nested.choice.to_string(), "first");
}

#[test]
fn real_default_property_does_not_shadow_the_builder_factory() {
    assert!(DefaultProperty::try_from(DefaultProperty::builder()).is_err());
    let value = DefaultProperty::try_from(DefaultProperty::builder().default("kept")).unwrap();
    assert_eq!(value.default, "kept");
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        serde_json::json!({"default": "kept"})
    );
    assert_eq!(
        serde_json::from_value::<DefaultProperty>(serde_json::json!({"default": "kept"}))
            .unwrap()
            .default,
        value.default
    );
}

#[test]
fn readable_bounds_keep_character_validation() {
    assert!("x".repeat(9_999).parse::<Bounded>().is_err());
    assert!("é".repeat(10_000).parse::<Bounded>().is_ok());
    assert_eq!(
        "!".parse::<Bounded>().unwrap_err().to_string(),
        "shorter than 10000 characters"
    );
    assert!(format!("{}!", "x".repeat(10_000))
        .parse::<Bounded>()
        .is_err());
}

#[test]
fn explicit_parse_types_keep_first_success_and_fallback_order() {
    for text in ["7", "true", "plain text"] {
        let value: ParseOrder = text.parse().unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), text);
    }
    assert_eq!(
        serde_json::to_value("7".parse::<ScalarValue>().unwrap()).unwrap(),
        7
    );
    assert_eq!(
        serde_json::to_value("true".parse::<ScalarValue>().unwrap()).unwrap(),
        true
    );
    assert!("invalid".parse::<ScalarValue>().is_err());
}

#[test]
fn map_wrapper_conversions_accept_standard_and_custom_hashers() {
    type AlternateMap = std::collections::HashMap<
        String,
        String,
        std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>,
    >;
    let json = serde_json::json!({"a": "one", "b": "two"});
    let ordinary: std::collections::HashMap<String, String> = StringMap::from(
        serde_json::from_value::<std::collections::HashMap<String, String>>(json.clone()).unwrap(),
    )
    .into();
    assert_eq!(serde_json::to_value(ordinary).unwrap(), json);
    let alternate: AlternateMap = serde_json::from_value::<StringMap>(json.clone())
        .unwrap()
        .into();
    assert_eq!(serde_json::to_value(alternate).unwrap(), json);
    let alternate: Option<AlternateMap> = serde_json::from_value::<OptionalStringMap>(json.clone())
        .unwrap()
        .into();
    assert_eq!(serde_json::to_value(alternate).unwrap(), json);
    let missing: Option<AlternateMap> =
        serde_json::from_value::<OptionalStringMap>(serde_json::Value::Null)
            .unwrap()
            .into();
    assert!(missing.is_none());
}
