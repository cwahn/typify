include!(concat!(env!("OUT_DIR"), "/variant_names.rs"));

#[test]
fn shortened_names_keep_wire_values_and_nonfirst_defaults() {
    assert_eq!(FunctionCallOutput::Text.to_string(), "TextContent");
    assert_eq!(FunctionCallOutput::Image.to_string(), "ImageContent");
    assert_eq!(
        OpaqueRecovery::ContainerAuto.to_string(),
        "ContainerAutoParam"
    );
    assert_eq!(
        OpaqueRecovery::LocalEnvironment.to_string(),
        "LocalEnvironmentParam"
    );
    assert_eq!(Recovery272::default(), Recovery272::Sharepoint);
    assert_eq!(Recovery272::Dropbox.to_string(), "connector_dropbox");
    for (wire, value) in [
        ("CreateFileOperation", ApplyPatchOperation::Create),
        ("DeleteFileOperation", ApplyPatchOperation::Delete),
        ("UpdateFileOperation", ApplyPatchOperation::Update),
    ] {
        assert_eq!(value.to_string(), wire);
        assert_eq!(serde_json::to_value(value).unwrap(), wire);
        assert_eq!(wire.parse::<ApplyPatchOperation>().unwrap(), value);
        assert_eq!(
            serde_json::from_value::<ApplyPatchOperation>(serde_json::json!(wire)).unwrap(),
            value
        );
    }
    assert_eq!(ApplyPatchOperation::default(), ApplyPatchOperation::Delete);
    assert_eq!(TransferState::default(), TransferState::Running);
    assert_eq!(TransferState::Queued.to_string(), "TransferStateQueued");
    assert_eq!(JobStatus::Done.to_string(), "JobDoneStatus");
    assert_eq!(KeywordOperation::Self_.to_string(), "OperationSelf");
}

#[test]
fn payloads_tagging_and_variant_order_keep_existing_semantics() {
    let value = PayloadPatchOperation::default();
    assert!(matches!(value, PayloadPatchOperation::Create(_)));
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        serde_json::json!({"create": "new"})
    );
    let create: PayloadPatchOperation =
        serde_json::from_value(serde_json::json!({"create": "new"})).unwrap();
    assert!(matches!(create, PayloadPatchOperation::Create(_)));
    assert_eq!(
        serde_json::to_value(create).unwrap(),
        serde_json::json!({"create": "new"})
    );
    let delete: PayloadPatchOperation =
        serde_json::from_value(serde_json::json!({"delete": "gone"})).unwrap();
    assert!(matches!(delete, PayloadPatchOperation::Delete(_)));
    assert_eq!(
        serde_json::to_value(delete).unwrap(),
        serde_json::json!({"delete": "gone"})
    );

    assert!(matches!(
        TaggedOperation::default(),
        TaggedOperation::Create
    ));
    assert_eq!(
        serde_json::to_value(TaggedOperation::default()).unwrap(),
        serde_json::json!({"kind": "CreateFileOperation"})
    );
    let wire = serde_json::json!({"kind": "DeleteFileOperation", "value": "body", "count": 2});
    let value: TaggedOperation = serde_json::from_value(wire.clone()).unwrap();
    assert!(matches!(value, TaggedOperation::Delete { .. }));
    assert_eq!(serde_json::to_value(value).unwrap(), wire);
}

#[test]
fn ambiguous_and_unicode_names_still_roundtrip() {
    assert_eq!(AccessMode::ReadOnly.to_string(), "ReadOnly");
    assert_eq!(AccessMode::ReadWrite.to_string(), "ReadWrite");
    assert_eq!(FlagMode::FlagModeA.to_string(), "FlagModeA");
    assert_eq!(WholeMode::WholeMode.to_string(), "WholeMode");
    for wire in ["🔥Operation", "🚀Operation"] {
        let value: UnicodeOperation = serde_json::from_value(serde_json::json!(wire)).unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), wire);
    }
}
