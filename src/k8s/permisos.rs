//! Qué puede hacer realmente tu credencial sobre un recurso.
//!
//! Se pregunta con `SelfSubjectAccessReview`: es la misma consulta que hace
//! `kubectl auth can-i`, la responde el API server evaluando el RBAC efectivo
//! y no necesita permisos especiales —cualquiera puede preguntar por sí mismo.
//!
//! Sirve para no ofrecer acciones que van a fallar, y de paso dice qué alcance
//! tiene de verdad una credencial.

use std::collections::HashMap;

use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
};
use kube::api::{Api, PostParams};
use kube::discovery::ApiResource;
use kube::Client;

use super::{K8sEvent, UiBridge};

/// Los verbos que la UI ofrece y por lo tanto necesita saber si están.
pub const VERBOS: &[&str] = &["delete", "patch", "update"];

/// Respuesta del API server para un (recurso, namespace).
#[derive(Clone, Debug, Default)]
pub struct Permisos {
    /// Verbo -> permitido. Ausente = todavía no se preguntó.
    pub por_verbo: HashMap<String, bool>,
}

impl Permisos {
    /// `true` si el API server dijo que no. Mientras no haya respuesta se
    /// asume que sí: es preferible ofrecer una acción que después falle a
    /// esconder una que el usuario sí podía hacer.
    pub fn prohibido(&self, verbo: &str) -> bool {
        self.por_verbo.get(verbo) == Some(&false)
    }
}

/// Clave de caché: el permiso depende del recurso y del namespace.
pub fn clave(ar: &ApiResource, ns: Option<&str>) -> String {
    format!("{}/{}", ns.unwrap_or(""), ar.plural)
}

pub async fn consultar(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    bridge: UiBridge,
) {
    let api: Api<SelfSubjectAccessReview> = Api::all(client);
    let mut por_verbo = HashMap::new();

    for verbo in VERBOS {
        let review = SelfSubjectAccessReview {
            spec: SelfSubjectAccessReviewSpec {
                resource_attributes: Some(ResourceAttributes {
                    group: Some(ar.group.clone()),
                    resource: Some(ar.plural.clone()),
                    verb: Some((*verbo).to_string()),
                    namespace: ns.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        match api.create(&PostParams::default(), &review).await {
            Ok(r) => {
                if let Some(st) = r.status {
                    por_verbo.insert((*verbo).to_string(), st.allowed);
                }
            }
            Err(e) => {
                // Si ni siquiera se puede preguntar, no se asume nada: la UI
                // sigue ofreciendo todo y el error aparece al intentar.
                tracing::debug!(error = %e, verbo, "permisos: no se pudo consultar");
                return;
            }
        }
    }

    tracing::info!(
        recurso = %ar.plural, ns = ?ns,
        permitidos = ?por_verbo.iter().filter(|(_, v)| **v).map(|(k, _)| k).collect::<Vec<_>>(),
        "permisos: consultados"
    );
    bridge.send(K8sEvent::Permisos {
        clave: clave(&ar, ns.as_deref()),
        permisos: Permisos { por_verbo },
    });
}
