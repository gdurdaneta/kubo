//! Columnas de un recurso custom, tal como las define su CRD.
//!
//! `kubectl get` muestra para un CRD las `additionalPrinterColumns` de su
//! versión almacenada (Sync/Health de una Application de Argo, Ready de un
//! Certificate…). Sin esto un recurso custom en la tabla es solo nombre y
//! edad. Acá se pide el CRD una vez por (cluster, kind) y se evalúan sus
//! JSONPath contra cada objeto.
//!
//! Solo se soporta el subconjunto de JSONPath que usan los CRD en la práctica:
//! `.a.b`, `.a[0].b`, `.a[*].b` y el filtro `.a[?(@.k=="v")].b`.

use kube::api::{Api, DynamicObject};
use kube::discovery::ApiResource;
use kube::Client;
use serde_json::Value;

use super::{K8sEvent, UiBridge};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ColumnaCrd {
    pub nombre: String,
    pub ruta: String,
    /// `string`, `integer`, `number`, `boolean` o `date`.
    pub tipo: String,
}

fn ar_crd() -> ApiResource {
    ApiResource {
        group: "apiextensions.k8s.io".into(),
        version: "v1".into(),
        api_version: "apiextensions.k8s.io/v1".into(),
        kind: "CustomResourceDefinition".into(),
        plural: "customresourcedefinitions".into(),
    }
}

/// Grupos servidos por el propio API server: no tienen CRD que consultar.
pub fn es_grupo_nativo(grupo: &str) -> bool {
    grupo.is_empty()
        || grupo == "k8s.io"
        || grupo.ends_with(".k8s.io")
        || grupo == "apps"
        || grupo == "batch"
        || grupo == "autoscaling"
        || grupo == "policy"
        || grupo == "extensions"
}

/// Columnas (prioridad 0) de la versión pedida del CRD, o de la almacenada.
pub fn parsear(crd: &Value, version: &str) -> Vec<ColumnaCrd> {
    let Some(versiones) = crd
        .get("spec")
        .and_then(|s| s.get("versions"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let elegida = versiones
        .iter()
        .find(|v| v.get("name").and_then(|n| n.as_str()) == Some(version))
        .or_else(|| {
            versiones
                .iter()
                .find(|v| v.get("storage").and_then(|s| s.as_bool()) == Some(true))
        });
    let Some(cols) = elegida
        .and_then(|v| v.get("additionalPrinterColumns"))
        .and_then(|c| c.as_array())
    else {
        return Vec::new();
    };
    cols.iter()
        .filter(|c| c.get("priority").and_then(|p| p.as_i64()).unwrap_or(0) == 0)
        .filter_map(|c| {
            let nombre = c.get("name")?.as_str()?.to_string();
            let ruta = c.get("jsonPath")?.as_str()?.to_string();
            // kubectl ya pone la edad por su cuenta; la tabla también.
            if nombre.eq_ignore_ascii_case("age") && ruta == ".metadata.creationTimestamp" {
                return None;
            }
            Some(ColumnaCrd {
                nombre,
                ruta,
                tipo: c
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("string")
                    .to_string(),
            })
        })
        .collect()
}

/// Un paso del JSONPath ya tokenizado.
#[derive(Debug, PartialEq)]
enum Paso<'a> {
    Campo(&'a str),
    Indice(usize),
    Todos,
    /// `[?(@.k=="v")]`
    Filtro(&'a str, &'a str),
}

fn tokenizar(ruta: &str) -> Option<Vec<Paso<'_>>> {
    let mut pasos = Vec::new();
    let mut resto = ruta.trim().strip_prefix('$').unwrap_or(ruta.trim());
    while !resto.is_empty() {
        if let Some(r) = resto.strip_prefix('.') {
            let fin = r.find(['.', '[']).unwrap_or(r.len());
            let (campo, r2) = r.split_at(fin);
            if campo.is_empty() {
                return None;
            }
            pasos.push(Paso::Campo(campo));
            resto = r2;
        } else if let Some(r) = resto.strip_prefix('[') {
            let cierre = r.find(']')?;
            let (interior, r2) = r.split_at(cierre);
            resto = &r2[1..];
            let interior = interior.trim();
            if interior == "*" {
                pasos.push(Paso::Todos);
            } else if let Ok(i) = interior.parse::<usize>() {
                pasos.push(Paso::Indice(i));
            } else if let Some(f) = interior.strip_prefix("?(@.") {
                // ?(@.type=="Ready")  ó  ?(@.type=='Ready')
                let f = f.strip_suffix(')')?;
                let (clave, valor) = f.split_once("==")?;
                let valor = valor.trim().trim_matches(|c| c == '"' || c == '\'');
                pasos.push(Paso::Filtro(clave.trim(), valor));
            } else {
                // Cualquier otra sintaxis (`['a']`, slices) no se soporta.
                let campo = interior.trim_matches(|c| c == '"' || c == '\'');
                if campo == interior {
                    return None;
                }
                pasos.push(Paso::Campo(campo));
            }
        } else {
            return None;
        }
    }
    Some(pasos)
}

fn aplicar<'v>(v: &'v Value, pasos: &[Paso<'_>]) -> Vec<&'v Value> {
    let mut actuales = vec![v];
    for paso in pasos {
        let mut siguientes = Vec::new();
        for a in actuales {
            match paso {
                Paso::Campo(c) => {
                    if let Some(x) = a.get(c) {
                        siguientes.push(x);
                    }
                }
                Paso::Indice(i) => {
                    if let Some(x) = a.get(i) {
                        siguientes.push(x);
                    }
                }
                Paso::Todos => {
                    if let Some(arr) = a.as_array() {
                        siguientes.extend(arr.iter());
                    }
                }
                Paso::Filtro(k, val) => {
                    if let Some(arr) = a.as_array() {
                        siguientes.extend(
                            arr.iter()
                                .filter(|e| e.get(k).map(escalar).as_deref() == Some(*val)),
                        );
                    }
                }
            }
        }
        actuales = siguientes;
        if actuales.is_empty() {
            break;
        }
    }
    actuales
}

fn escalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Array(a) => a.iter().map(escalar).collect::<Vec<_>>().join(","),
        otro => otro.to_string(),
    }
}

/// Evalúa la ruta sobre el objeto. Varios resultados se unen con coma, como
/// hace kubectl. `None` si la ruta no se entiende o no matchea nada.
pub fn evaluar(ruta: &str, obj: &Value) -> Option<String> {
    let pasos = tokenizar(ruta)?;
    let vals = aplicar(obj, &pasos);
    if vals.is_empty() {
        return None;
    }
    Some(
        vals.iter()
            .map(|v| escalar(v))
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// Valor de una columna para la tabla: las fechas van como edad relativa.
pub fn celda(col: &ColumnaCrd, obj: &Value) -> String {
    let Some(v) = evaluar(&col.ruta, obj) else {
        return String::new();
    };
    if col.tipo == "date" {
        return crate::columns::edad_desde_rfc3339(&v);
    }
    v
}

/// Pide el CRD del recurso y manda sus columnas al hilo de UI. Un 404 o un
/// 403 solo significan que no hay columnas: no es un error para el usuario.
pub async fn obtener(client: Client, ar: ApiResource, token: u64, bridge: UiBridge) {
    let nombre = format!("{}.{}", ar.plural, ar.group);
    let api: Api<DynamicObject> = Api::all_with(client, &ar_crd());
    let columnas = match api.get(&nombre).await {
        Ok(crd) => parsear(&crd.data, &ar.version),
        Err(e) => {
            tracing::debug!(crd = %nombre, error = %e, "printer: sin columnas");
            Vec::new()
        }
    };
    tracing::info!(crd = %nombre, n = columnas.len(), "printer: columnas del CRD");
    bridge.send(K8sEvent::ColumnasCrd {
        token,
        clave: ar.kind.clone(),
        columnas,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rutas_simples() {
        let o = json!({"status": {"sync": {"status": "Synced"}, "n": 3, "ok": true}});
        assert_eq!(
            evaluar(".status.sync.status", &o).as_deref(),
            Some("Synced")
        );
        assert_eq!(evaluar(".status.n", &o).as_deref(), Some("3"));
        assert_eq!(evaluar(".status.ok", &o).as_deref(), Some("true"));
        assert_eq!(evaluar(".status.nada", &o), None);
        assert_eq!(evaluar("$.status.n", &o).as_deref(), Some("3"));
    }

    #[test]
    fn indices_y_comodin() {
        let o = json!({"spec": {"hosts": ["a", "b"], "items": [{"n": 1}, {"n": 2}]}});
        assert_eq!(evaluar(".spec.hosts[0]", &o).as_deref(), Some("a"));
        assert_eq!(evaluar(".spec.hosts", &o).as_deref(), Some("a,b"));
        assert_eq!(evaluar(".spec.items[*].n", &o).as_deref(), Some("1,2"));
        assert_eq!(evaluar(".spec.items[5].n", &o), None);
    }

    #[test]
    fn filtro_por_condicion() {
        let o = json!({"status": {"conditions": [
            {"type": "Progressing", "status": "True"},
            {"type": "Ready", "status": "False", "reason": "Pending"},
        ]}});
        assert_eq!(
            evaluar(r#".status.conditions[?(@.type=="Ready")].status"#, &o).as_deref(),
            Some("False")
        );
        assert_eq!(
            evaluar(".status.conditions[?(@.type=='Ready')].reason", &o).as_deref(),
            Some("Pending")
        );
        assert_eq!(
            evaluar(r#".status.conditions[?(@.type=="X")].status"#, &o),
            None
        );
    }

    #[test]
    fn sintaxis_rara_no_revienta() {
        let o = json!({"a": 1});
        assert_eq!(evaluar(".a[1:2]", &o), None);
        assert_eq!(evaluar("a", &o), None);
        assert_eq!(evaluar(".", &o), None);
    }

    #[test]
    fn parsea_columnas_de_la_version_almacenada() {
        let crd = json!({"spec": {"versions": [
            {"name": "v1alpha1", "storage": false, "additionalPrinterColumns": [
                {"name": "Viejo", "jsonPath": ".x", "type": "string"}]},
            {"name": "v1", "storage": true, "additionalPrinterColumns": [
                {"name": "Sync Status", "jsonPath": ".status.sync.status", "type": "string"},
                {"name": "Age", "jsonPath": ".metadata.creationTimestamp", "type": "date"},
                {"name": "Oculta", "jsonPath": ".y", "type": "string", "priority": 1}]},
        ]}});
        let cols = parsear(&crd, "v1");
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].nombre, "Sync Status");
        // Sin versión conocida cae en la storage.
        assert_eq!(parsear(&crd, "v9").len(), 1);
        assert_eq!(parsear(&crd, "v1alpha1")[0].nombre, "Viejo");
    }

    #[test]
    fn grupos_nativos() {
        assert!(es_grupo_nativo(""));
        assert!(es_grupo_nativo("apps"));
        assert!(es_grupo_nativo("networking.k8s.io"));
        assert!(!es_grupo_nativo("argoproj.io"));
        assert!(!es_grupo_nativo("cert-manager.io"));
    }
}
