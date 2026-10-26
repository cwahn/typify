#[cfg(test)]
mod tests {
    use super::{after, builder_api};
    use serde::{de::DeserializeOwned, Deserialize, Serialize};
    use serde_json::{json, Value};

    // The old layout and exact Serde attributes are the behavioral baseline.
    // These are intentionally not an improved representation of anyOf.
    mod before {
        use super::{after, Deserialize, Serialize};

        #[derive(Deserialize, Serialize)]
        pub(super) struct ActualModelIdsCompaction {
            #[serde(flatten, skip_serializing_if = "Option::is_none")]
            pub(super) subtype_0: Option<after::ModelIdsResponses>,
            #[serde(flatten, skip_serializing_if = "Option::is_none")]
            pub(super) subtype_1: Option<String>,
            #[serde(flatten, skip_serializing_if = "Option::is_none")]
            pub(super) subtype_2: Option<()>,
        }

        #[derive(Deserialize, Serialize)]
        pub(super) struct ObjectAny {
            #[serde(flatten, skip_serializing_if = "Option::is_none")]
            subtype_0: Option<after::RequestId>,
            #[serde(flatten, skip_serializing_if = "Option::is_none")]
            subtype_1: Option<after::ResponseBody>,
        }

        #[derive(Deserialize, Serialize)]
        pub(super) struct ObjectAll {
            enabled: bool,
            #[serde(rename = "request-id")]
            request_id: String,
        }
    }

    fn roundtrip<T: DeserializeOwned + Serialize>(wire: Value) -> Result<Value, String> {
        serde_json::from_value::<T>(wire)
            .and_then(serde_json::to_value)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn approved_fields_and_all_three_builder_setters_compile() {
        let value: after::ActualModelIdsCompaction = after::ActualModelIdsCompaction::builder()
            .model_ids_responses(None::<after::ModelIdsResponses>)
            .string(None::<String>)
            .null(None::<()>)
            .try_into()
            .unwrap();
        assert!(value.model_ids_responses.is_none());
        assert!(value.string.is_none());
        assert!(value.null.is_none());
        let rebuilt: after::ActualModelIdsCompaction =
            after::builder::ActualModelIdsCompaction::from(value)
                .try_into()
                .unwrap();
        assert_eq!(serde_json::to_value(rebuilt).unwrap(), json!({}));
    }

    #[test]
    fn scalar_flatten_acceptance_and_existing_failures_are_unchanged() {
        for wire in [
            Value::Null,
            json!("model-a"),
            json!("other"),
            json!(0),
            json!(false),
            json!([]),
            json!({}),
            json!({"arbitrary": true}),
            json!({"model_ids_responses": "model-a", "string": "other", "null": null}),
        ] {
            assert_eq!(
                roundtrip::<after::ActualModelIdsCompaction>(wire.clone()),
                roundtrip::<before::ActualModelIdsCompaction>(wire.clone()),
                "{wire}",
            );
        }
        // Scalars are valid per the input schema, but the existing flattened
        // representation cannot deserialize them. This naming fix must not
        // silently expand into a Serde conformance change.
        assert!(roundtrip::<after::ActualModelIdsCompaction>(json!("model-a")).is_err());
        assert!(roundtrip::<after::ActualModelIdsCompaction>(Value::Null).is_err());
        for (reference, string, null) in [
            (
                Some(serde_json::from_value(json!("model-a")).unwrap()),
                None,
                None,
            ),
            (None, Some("other".to_string()), None),
            (None, None, Some(())),
        ] {
            let plain_string = string.is_some();
            let after = after::ActualModelIdsCompaction {
                model_ids_responses: reference.clone(),
                string: string.clone(),
                null,
            };
            let before = before::ActualModelIdsCompaction {
                subtype_0: reference,
                subtype_1: string,
                subtype_2: null,
            };
            let after = serde_json::to_value(after).map_err(|error| error.to_string());
            let before = serde_json::to_value(before).map_err(|error| error.to_string());
            if plain_string {
                assert!(after.is_err());
            }
            assert_eq!(after, before);
        }
    }

    #[test]
    fn object_anyof_and_allof_keep_real_json_keys_and_accept_reject_behavior() {
        let wire = json!({"request-id": "request-1", "enabled": true});
        for output in [
            roundtrip::<after::ObjectAny>(wire.clone()).unwrap(),
            roundtrip::<after::ObjectAll>(wire.clone()).unwrap(),
        ] {
            assert_eq!(output, wire);
            let keys = output.as_object().unwrap();
            for synthetic in ["request_id", "response_body", "subtype_0", "subtype_1"] {
                assert!(!keys.contains_key(synthetic));
            }
        }
        let value: after::ObjectAny = after::ObjectAny::builder()
            .request_id(Some(after::RequestId {
                enabled: None,
                request_id: "request-1".to_string(),
            }))
            .response_body(Some(after::ResponseBody {
                enabled: true,
                request_id: None,
            }))
            .try_into()
            .unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), wire);
        let value: after::ObjectAll = after::ObjectAll::builder()
            .request_id("request-1")
            .enabled(true)
            .try_into()
            .unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), wire);

        for wire in [
            wire,
            json!({"request-id": "request-1"}),
            json!({"enabled": true}),
            json!({}),
            json!({"request-id": 7, "enabled": "bad"}),
            json!({"request-id": "request-1", "enabled": false, "extra": 1}),
            json!({"request_id": "request-1", "response_body": {"enabled": true}}),
            Value::Null,
            json!("scalar"),
            json!([]),
            json!(1),
        ] {
            assert_eq!(
                roundtrip::<after::ObjectAny>(wire.clone()),
                roundtrip::<before::ObjectAny>(wire.clone()),
                "anyOf {wire}"
            );
            assert_eq!(
                roundtrip::<after::ObjectAll>(wire.clone()),
                roundtrip::<before::ObjectAll>(wire.clone()),
                "allOf {wire}"
            );
        }
        assert!(roundtrip::<after::ObjectAll>(json!({"enabled": true})).is_err());
        assert!(roundtrip::<after::ObjectAny>(json!("scalar")).is_err());
    }

    #[test]
    fn builder_factories_and_standard_trait_methods_remain_callable() {
        use std::{
            any::{Any, TypeId},
            borrow::{Borrow, BorrowMut},
            fmt,
        };

        // The concrete review repro: `default` must not become an inherent
        // setter that hides the Default::default call inside this factory.
        let _: builder_api::DefaultFactory = builder_api::DefaultFactory::builder()
            .default_value(None::<builder_api::Default>)
            .try_into()
            .unwrap();
        let _: builder_api::TryIntoFactory = builder_api::TryIntoFactory::builder()
            .try_into_value(None::<builder_api::TryInto>)
            .try_into()
            .unwrap();

        type ApiBuilder = builder_api::builder::BuilderApiFields;
        let builder = builder_api::BuilderApiFields::builder()
            .default_value(None::<builder_api::Default>)
            .default_value_3(None::<builder_api::DefaultValue>)
            .default_value_2(None::<builder_api::DefaultValue2>)
            .clone_value(None::<builder_api::Clone>)
            .clone_from_value(None::<builder_api::CloneFrom>)
            .from_value(None::<builder_api::From>)
            .try_from_value(None::<builder_api::TryFrom>)
            .into_value(None::<builder_api::Into>)
            .try_into_value(None::<builder_api::TryInto>)
            .fmt_value(None::<builder_api::Fmt>)
            .to_owned_value(None::<builder_api::ToOwned>)
            .clone_into_value(None::<builder_api::CloneInto>)
            .borrow_value(None::<builder_api::Borrow>)
            .borrow_mut_value(None::<builder_api::BorrowMut>)
            .type_id_value(None::<builder_api::TypeId>);
        let mut cloned = builder.clone();
        cloned.clone_from(&builder);
        let mut owned = builder.to_owned();
        builder.clone_into(&mut owned);
        let borrowed: &ApiBuilder = builder.borrow();
        assert_eq!(borrowed.type_id(), TypeId::of::<ApiBuilder>());
        let borrowed_mut: &mut ApiBuilder = owned.borrow_mut();
        borrowed_mut.clone_from(&cloned);

        struct DebugViaMethod<'a>(&'a ApiBuilder);
        impl fmt::Display for DebugViaMethod<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                use fmt::Debug;
                self.0.fmt(formatter)
            }
        }
        assert!(format!("{}", DebugViaMethod(&builder)).contains("BuilderApiFields"));

        let model: builder_api::BuilderApiFields = cloned.try_into().unwrap();
        let rebuilt = ApiBuilder::from(model);
        let model: builder_api::BuilderApiFields = rebuilt.try_into().unwrap();
        fn try_builder_from<T>(value: T) -> Result<ApiBuilder, <ApiBuilder as TryFrom<T>>::Error>
        where
            ApiBuilder: TryFrom<T>,
        {
            ApiBuilder::try_from(value)
        }
        let builder = try_builder_from(model).unwrap();

        struct BuilderWrapper(ApiBuilder);
        impl From<ApiBuilder> for BuilderWrapper {
            fn from(builder: ApiBuilder) -> Self {
                Self(builder)
            }
        }
        let wrapped: BuilderWrapper = builder.into();
        let _: builder_api::BuilderApiFields = wrapped.0.try_into().unwrap();
        let _: ApiBuilder = ApiBuilder::default();
    }
}
