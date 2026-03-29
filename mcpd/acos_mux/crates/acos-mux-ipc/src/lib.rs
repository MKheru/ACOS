//! Inter-process communication protocol and transport.

pub mod codec;
pub mod messages;
pub mod transport;
pub mod acos;
pub mod mcp_bridge;

pub use codec::{CodecError, decode, encode, read_message, write_message};
pub use messages::{
    ClientMessage, PROTOCOL_VERSION, PaneEntry, ServerMessage, SessionEntry, SplitDirection,
};
pub use transport::{Listener, ReadWrite, SshStream, Transport, TransportError};
pub use mcp_bridge::{McpError, McpNotification, McpRequest, McpResponse};
