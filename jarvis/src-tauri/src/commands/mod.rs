mod executor;
mod matcher;
mod open_target;
mod router;
mod tools;

pub use executor::{execute_command_with_context, ToolCallContext};
pub use matcher::match_command;
pub use router::try_route_and_execute;
#[allow(unused_imports)]
pub use tools::{execute_registered_script, execute_tool, load_tool, substitute_tool_args};
