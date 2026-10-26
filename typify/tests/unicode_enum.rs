// Copyright 2026 Oxide Computer Company

#![deny(warnings)]

typify::import_types!("tests/unicode-enum.json");

#[test]
fn authentic_hugging_face_enum_roundtrips() {
    let variants = [
        ("🔥", HuggingFaceReaction::U1F525),
        ("🚀", HuggingFaceReaction::U1F680),
        ("👀", HuggingFaceReaction::U1F440),
        ("❤️", HuggingFaceReaction::U2764UFE0F),
        ("🤗", HuggingFaceReaction::U1F917),
        ("😎", HuggingFaceReaction::U1F60E),
        ("➕", HuggingFaceReaction::U2795),
        ("🧠", HuggingFaceReaction::U1F9E0),
        ("👍", HuggingFaceReaction::U1F44D),
        ("🤝", HuggingFaceReaction::U1F91D),
        ("😔", HuggingFaceReaction::U1F614),
        ("🤯", HuggingFaceReaction::U1F92F),
    ];
    for (raw, variant) in variants {
        let value = serde_json::json!(raw);
        assert_eq!(serde_json::to_value(&variant).unwrap(), value);
        let decoded: HuggingFaceReaction = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), value);
        assert_eq!(decoded.to_string(), raw);
        assert_eq!(raw.parse::<HuggingFaceReaction>().unwrap().to_string(), raw);
    }
}

#[test]
fn adversarial_values_roundtrip_without_normalizing_wire_values() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("unicode-enum.json")).unwrap();
    for value in schema["definitions"]["Adversarial"]["enum"]
        .as_array()
        .unwrap()
    {
        let variant: Adversarial = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&variant).unwrap(), *value);
        assert_eq!(variant.to_string(), value.as_str().unwrap());
    }
    assert_eq!(serde_json::to_value(Adversarial::U1f525).unwrap(), "U1F525");
    assert_eq!(serde_json::to_value(Adversarial::U1F525).unwrap(), "🔥");
    assert_eq!(serde_json::to_value(Adversarial::U21).unwrap(), "U21");
    assert_eq!(serde_json::to_value(Adversarial::UU21).unwrap(), "uU21");
    assert_eq!(serde_json::to_value(Adversarial::UUU21).unwrap(), "!");
}

#[test]
#[should_panic(expected = "Failed to make unique variant names")]
fn duplicate_schema_values_keep_existing_rejection() {
    let schema = serde_json::from_value(serde_json::json!({
        "title": "DuplicateValues",
        "type": "string",
        "enum": ["🔥", "🔥", "🚀"]
    }))
    .unwrap();
    typify::TypeSpace::default()
        .add_root_schema(schema)
        .unwrap();
}
