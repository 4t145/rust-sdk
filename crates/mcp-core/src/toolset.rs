use std::{borrow::Borrow, collections::HashMap, ops::Deref, sync::Arc};

use serde_json::Value;

use crate::{
    Content, Tool,
    handler::{DynToolHandler, ToolHandler},
};

#[derive(Default)]
pub struct ToolSet {
    tools: HashMap<&'static str, DynToolHandler>,
}

impl std::fmt::Debug for ToolSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.tools.iter().map(|(k, v)| (*k, v.description())))
            .finish()
    }
}

impl ToolSet {
    pub fn add_tool<H: ToolHandler>(&mut self, tool: H) -> Option<DynToolHandler> {
        self.tools.insert(tool.name(), DynToolHandler::new(tool))
    }
    pub fn add_boxed_tool(&mut self, tool: Box<dyn ToolHandler>) -> Option<DynToolHandler> {
        self.tools
            .insert(tool.name(), DynToolHandler::new_boxed(tool))
    }
    pub fn remove_tool<S>(&mut self, name: &S) -> std::option::Option<DynToolHandler>
    where
        &'static str: Borrow<S>,
        S: std::hash::Hash + Eq + ?Sized,
    {
        self.tools.remove(name)
    }
    pub fn get_tool<S>(&self, name: &S) -> Option<&DynToolHandler>
    where
        &'static str: Borrow<S>,
        S: std::hash::Hash + Eq + ?Sized,
    {
        self.tools.get(name)
    }

    pub fn extend(&mut self, tool_set: ToolSet) {
        self.tools.extend(tool_set.tools);
    }

    pub async fn call(&self, name: &str, params: Value) -> Result<Vec<Content>, crate::ToolError> {
        let handler = self
            .get_tool(name)
            .ok_or(crate::ToolError::NotFound(name.to_string()))?;
        let result = handler.call(params).await?;
        Ok(result)
    }

    pub fn list_all(&self) -> Vec<Tool> {
        self.tools
            .values()
            .map(|handler| Tool::from_handler(handler.deref()))
            .collect()
    }
}
