use crate::model::ElicitRequestParams;

impl ElicitRequestParams {
    #[cfg(feature = "schemars")]
    pub fn with_typed_schema<T: schemars::JsonSchema>(message: impl Into<String>) -> Self {
        let mut schema_config = schemars::r#gen::SchemaSettings::default();
        schema_config.meta_schema = None;
        let requested_schema = schema_config.into_generator().root_schema_for::<T>();
        let requested_schema = serde_json::to_value(requested_schema)
            .expect("json schema always should be a valid json value");
        Self {
            message: message.into(),
            requested_schema,
        }
    }
}
