//! Enmascara lo que suele filtrarse en logs (tokens, contraseñas, claves)
//! antes de copiarlos al portapapeles o compartirlos.
//!
//! No es infalible —un secreto sin forma reconocible pasa igual—, pero cubre
//! los formatos que más aparecen: Bearer/JWT, claves de AWS, `password=`,
//! credenciales en URLs y cabeceras Authorization.

/// Devuelve el texto con los valores sensibles reemplazados por `«redactado»`
/// y cuántos reemplazos hizo.
pub fn redactar(texto: &str) -> (String, usize) {
    let mut out = String::with_capacity(texto.len());
    let mut n = 0;
    for linea in texto.split_inclusive('\n') {
        let (l, k) = redactar_linea(linea);
        out.push_str(&l);
        n += k;
    }
    (out, n)
}

const MARCA: &str = "«redactado»";

fn redactar_linea(l: &str) -> (String, usize) {
    let mut s = l.to_string();
    let mut n = 0;
    // 1. JWT: tres bloques base64url separados por puntos que arrancan con eyJ.
    n += reemplazar_tokens(&mut s, |t| {
        t.starts_with("eyJ") && t.matches('.').count() == 2 && t.len() > 30
    });
    // 2. Claves de acceso AWS y tokens con prefijo conocido.
    n += reemplazar_tokens(&mut s, |t| {
        (t.starts_with("AKIA") && t.len() == 20)
            || t.starts_with("ghp_")
            || t.starts_with("github_pat_")
            || t.starts_with("xoxb-")
            || t.starts_with("xoxp-")
            || t.starts_with("sk-")
                && t.len() > 20
                && t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || t.starts_with("glpat-")
            || t.starts_with("hvs.")
    });
    // 3. `Bearer xxx`, `Basic xxx` y `Authorization: xxx` (cuando el valor no
    //    lleva esquema, que ya lo cubre la regla anterior).
    n += tras_palabra(&mut s, "Bearer ");
    n += tras_palabra(&mut s, "Basic ");
    for pref in ["Authorization: ", "authorization: "] {
        if let Some(i) = s.find(pref) {
            let resto = &s[i + pref.len()..];
            let con_esquema = resto.starts_with("Bearer ")
                || resto.starts_with("Basic ")
                || resto.starts_with(MARCA);
            if !con_esquema {
                n += tras_palabra(&mut s, pref);
            }
        }
    }
    // 4. clave=valor con nombres sensibles (password, passwd, secret, token,
    //    api_key, apikey, access_key), con `=` o `:`; también entre comillas.
    for clave in [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "api-key",
        "access_key",
        "private_key",
        "client_secret",
    ] {
        n += tras_clave(&mut s, clave);
    }
    // 5. Credenciales en URLs: esquema://usuario:clave@host.
    n += credencial_en_url(&mut s);
    (s, n)
}

/// Reemplaza cada token (separado por espacios/comillas) que cumpla `es`.
fn reemplazar_tokens(s: &mut String, es: impl Fn(&str) -> bool) -> usize {
    let mut n = 0;
    let mut out = String::with_capacity(s.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String, n: &mut usize| {
        if !token.is_empty() {
            if es(token) {
                out.push_str(MARCA);
                *n += 1;
            } else {
                out.push_str(token);
            }
            token.clear();
        }
    };
    for c in s.chars() {
        if c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}'
            )
        {
            flush(&mut token, &mut out, &mut n);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    flush(&mut token, &mut out, &mut n);
    *s = out;
    n
}

/// Lo que sigue a `prefijo` hasta el próximo espacio, comilla o fin.
fn tras_palabra(s: &mut String, prefijo: &str) -> usize {
    let mut n = 0;
    let mut desde = 0;
    while let Some(i) = s[desde..].find(prefijo) {
        let ini = desde + i + prefijo.len();
        let fin = s[ini..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
            .map(|k| ini + k)
            .unwrap_or(s.len());
        if fin > ini && &s[ini..fin] != MARCA {
            s.replace_range(ini..fin, MARCA);
            n += 1;
        }
        desde = ini + MARCA.len().min(s.len() - ini);
    }
    n
}

/// `clave=valor`, `clave: valor`, `"clave":"valor"` (sin distinguir mayúsculas).
fn tras_clave(s: &mut String, clave: &str) -> usize {
    let mut n = 0;
    let lower = s.to_lowercase();
    let mut desde = 0;
    let mut rangos: Vec<(usize, usize)> = Vec::new();
    while let Some(i) = lower[desde..].find(clave) {
        let k = desde + i;
        // Tiene que ser palabra entera al inicio (evita `tokenizer`).
        let antes_ok = k == 0 || !lower.as_bytes()[k - 1].is_ascii_alphanumeric();
        let mut j = k + clave.len();
        // Permite `_`/`"` de cierre y separador `=` o `:` con espacios.
        while j < s.len() && matches!(s.as_bytes()[j], b'"' | b'\'') {
            j += 1;
        }
        let sep_ok = j < s.len() && matches!(s.as_bytes()[j], b'=' | b':');
        if antes_ok && sep_ok {
            j += 1;
            while j < s.len() && matches!(s.as_bytes()[j], b' ' | b'"' | b'\'') {
                j += 1;
            }
            let fin = s[j..]
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '&')
                .map(|x| j + x)
                .unwrap_or(s.len());
            if fin > j && &s[j..fin] != MARCA {
                rangos.push((j, fin));
            }
        }
        desde = k + clave.len();
    }
    for (a, b) in rangos.into_iter().rev() {
        s.replace_range(a..b, MARCA);
        n += 1;
    }
    n
}

fn credencial_en_url(s: &mut String) -> usize {
    let mut n = 0;
    let mut desde = 0;
    while let Some(i) = s[desde..].find("://") {
        let ini = desde + i + 3;
        let Some(arroba) = s[ini..].find('@') else {
            break;
        };
        let seg = &s[ini..ini + arroba];
        if let Some(dp) = seg.find(':') {
            if !seg.contains(char::is_whitespace) && !seg.contains('/') {
                let a = ini + dp + 1;
                let b = ini + arroba;
                if &s[a..b] != MARCA {
                    s.replace_range(a..b, MARCA);
                    n += 1;
                }
            }
        }
        desde = ini;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::redactar;

    #[test]
    fn enmascara_tokens_y_claves() {
        let (t, n) = redactar(
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcdefghijklmnop fin",
        );
        assert_eq!(t, "Authorization: Bearer «redactado» fin");
        assert!(n >= 1);
        let (t, _) = redactar("authorization: abc.def x-api: 1");
        assert_eq!(t, "authorization: «redactado» x-api: 1");
        let (t, _) = redactar("db=postgres://app:S3cr3t@db:5432/x password=hunter2 token: abc123 \"api_key\": \"k-1\"");
        assert!(t.contains("postgres://app:«redactado»@db:5432/x"), "{t}");
        assert!(t.contains("password=«redactado»"), "{t}");
        assert!(t.contains("token: «redactado»"), "{t}");
        assert!(t.contains("\"api_key\": \"«redactado»\""), "{t}");
        let (t, _) = redactar("aws AKIAIOSFODNN7EXAMPLE y ghp_abcdef0123456789");
        assert_eq!(t, "aws «redactado» y «redactado»");
    }

    #[test]
    fn deja_en_paz_lo_normal() {
        let (t, n) = redactar("GET /api/tokenizer 200 12ms user=juan secretaria=ana\n");
        assert_eq!(n, 0);
        assert_eq!(t, "GET /api/tokenizer 200 12ms user=juan secretaria=ana\n");
    }
}
