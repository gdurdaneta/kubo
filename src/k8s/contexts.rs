//! Lectura del kubeconfig. No toca la red: solo enumera lo declarado.

use anyhow::{Context as _, Result};
use kube::config::Kubeconfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextInfo {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub namespace: Option<String>,
}

/// Devuelve los contextos del kubeconfig y cuál está marcado como actual.
pub fn load() -> Result<(Vec<ContextInfo>, Option<String>)> {
    let cfg = Kubeconfig::read().context("no se pudo leer el kubeconfig")?;
    let list = cfg
        .contexts
        .iter()
        .filter_map(|c| {
            let ctx = c.context.as_ref()?;
            Some(ContextInfo {
                name: c.name.clone(),
                cluster: ctx.cluster.clone(),
                user: ctx.user.clone().unwrap_or_default(),
                namespace: ctx.namespace.clone(),
            })
        })
        .collect();
    Ok((list, cfg.current_context))
}
