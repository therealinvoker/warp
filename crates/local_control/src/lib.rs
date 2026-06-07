//! Shared protocol, discovery, authentication, and client types for local Warp control.
pub mod auth;
pub mod catalog;
pub mod client;
pub mod discovery;
pub mod protocol;
pub mod selection;
pub mod selectors;

pub use auth::{
    AuthToken, CredentialGrant, CredentialRequest, ScopedCredential, TerminalSessionProof,
    TerminalSessionProofRegistry,
};
pub use catalog::{ActionImplementationStatus, ActionKind, ActionMetadata, TargetScope};
pub use discovery::{
    ControlEndpoint, InstanceId, InstanceRecord, RegisteredInstance, discovery_dir,
};
pub use protocol::{
    Action, ActionInspectResult, ActionListResult, ActionNameParams, ActiveTargetChain,
    AppearanceStateResult, BindingNameParams, BlockListParams, BlockListResult, BlockSummary,
    BooleanValueParams, ColorValueParams, ControlError, ControlResponse, Direction,
    DirectionParams, EmptyParams, ErrorCode, ErrorResponseEnvelope, FileOpenParams, KeyParams,
    KeyValueParams, KeybindingGetParams, KeybindingGetResult, KeybindingListParams,
    KeybindingListResult, KeybindingSummary, LimitParams, NamespaceParams, PROTOCOL_VERSION,
    PageQueryParams, QueryParams, RenameParams, RequestEnvelope, ResizeParams, ResponseEnvelope,
    SettingGetParams, SettingGetResult, SettingListParams, SettingListResult, SettingSummary,
    TabActivateParams, TabActivationMode, TabCloseMode, TabCloseParams, TabCreateParams, TabType,
    TextParams, ThemeListResult, ThemeNameParams, ThemeStateResult, ThemeSummary,
};
pub use selectors::{PaneSelector, SessionSelector, TabSelector, TargetSelector, WindowSelector};
