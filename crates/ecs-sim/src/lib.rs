//! Circuit simulation engine (spec section 4).
//!
//! Modified Nodal Analysis in charge-oriented form, `i(x, t) + dq(x)/dt = 0`,
//! solved per time step with Newton-Raphson. Components are compiled from
//! their [`ecs_model::ComponentDef`] expressions into autodiff tapes; nothing
//! here is specific to any component.

mod circuit;
pub mod linear;
pub mod live;

pub use circuit::{
    Circuit, Instance, Method, Netlist, OpMethod, Options, SimError, analysis, flatten,
};
