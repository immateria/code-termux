#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub(crate) struct McpServerId(String);

impl McpServerId {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_ascii_lowercase()))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub(crate) struct McpToolId {
    server: McpServerId,
    tool: String,
}

impl McpToolId {
    pub(crate) fn parse(server: &str, tool: &str) -> Option<Self> {
        let server = McpServerId::parse(server)?;
        let tool = tool.trim();
        if tool.is_empty() {
            return None;
        }
        Some(Self {
            server,
            tool: tool.to_ascii_lowercase(),
        })
    }

    pub(crate) fn parse_spec(spec: &str) -> Option<Self> {
        let mut trimmed = spec.trim();
        if trimmed.is_empty() {
            return None;
        }

        if let Some(rest) = trimmed.strip_prefix("mcp://") {
            trimmed = rest;
        }

        if let Some((server, tool)) = trimmed.split_once("::") {
            return Self::parse(server, tool);
        }

        let (server, tool) = trimmed.split_once('/')?;
        Self::parse(server, tool)
    }

    pub(crate) fn into_parts(self) -> (String, String) {
        (self.server.0, self.tool)
    }
}
