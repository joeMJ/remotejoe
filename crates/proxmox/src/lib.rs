//! Proxmox VE: REST-Schnittstelle (API-Token oder Ticket), Liste der VMs und
//! Container, Start/Stopp/Neustart und Konsole über den VNC-Websocket.

pub mod api;
pub mod net;

pub use api::{Action, Auth, Client, ConsoleBridge, Guest, GuestInfo, GuestKind, ProxmoxError};
pub use net::{Base, CertCheck};
