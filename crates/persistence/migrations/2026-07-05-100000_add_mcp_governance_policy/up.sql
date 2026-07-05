-- Single-row snapshot of the effective MCP governance policy, so enforcement
-- can be applied at startup (before the first server fetch) and while offline.
CREATE TABLE mcp_governance_policy (
    id INTEGER PRIMARY KEY NOT NULL,
    policy_json TEXT NOT NULL
);
