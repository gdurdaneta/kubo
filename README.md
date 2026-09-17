# kubo

Cliente de escritorio para Kubernetes, nativo. Sin Electron, sin webview.

Rust + [egui](https://github.com/emilk/egui) sobre wgpu (Vulkan) y
[kube-rs](https://kube.rs) para hablar con el API server.

## Qué hace hoy

- **Multi-contexto**: lee `~/.kube/config` y cambia de cluster en caliente.
  Soporta los exec plugins (`aws eks get-token`, gke-gcloud-auth-plugin, OIDC).
- **Discovery dinámico**: la navegación se arma con lo que el cluster sirve,
  CRDs incluidos. Nada de listas hardcodeadas.
- **Detección de operadores**: si el cluster tiene Istio, Argo CD, Gateway API,
  cert-manager, Prometheus Operator, Flux, KEDA, External Secrets, MetalLB o el
  operador de RabbitMQ, aparecen como secciones propias del sidebar (Gateway
  API y MetalLB anidados dentro de Network, como en Lens). La detección es por
  grupos de API, así que solo se ofrece lo realmente instalado.
- **Métricas**: CPU y memoria en Pods y Nodes desde `metrics.k8s.io`, con
  porcentaje sobre lo asignable en los nodos y un sparkline de los últimos
  10 minutos en el detalle. Si el cluster no tiene metrics-server, las
  columnas no aparecen.
- **Tablas en vivo**: cada vista es un `watch` del API server, no un polling.
  Virtualizadas — se dibujan solo las filas visibles.
- **Columnas por Kind**: Pods, Deployments, Services, Nodes, PVCs, Ingresses,
  Jobs, CronJobs… con el mismo criterio que `kubectl get` (incluido el estado
  real del pod: `CrashLoopBackOff` gana sobre la fase `Running`).
- **Detalle**: resumen legible, YAML releído del API server (sin
  `managedFields`) y eventos del objeto resueltos por UID.
- **Logs en streaming**: follow, contenedor anterior, filtro y coloreado por
  nivel. Tope de 5.000 líneas en memoria.
- **Shell dentro del pod**: exec con PTY sobre WebSocket y emulador de
  terminal embebido (vt100) — colores, cursor, resize, Ctrl+C.
- **Acciones**: editar el YAML y aplicarlo, escalar, rollout restart y borrar,
  siempre con modal de confirmación. Aplicar muestra el diff contra la copia
  del API server y rechaza cambios de nombre o namespace: en Kubernetes los
  recursos no se renombran. En contextos que parecen producción, borrar,
  aplicar o escalar a cero exigen teclear el nombre del recurso.
- **Selección múltiple**: casillas por fila (espacio, ctrl+clic, casilla
  maestra) y acciones por lote —reiniciar, escalar, borrar— con una sola
  confirmación que lista los objetivos.
- **Logs y shell desde el workload**: en un Deployment, StatefulSet o
  DaemonSet, Logs mezcla todos sus pods (con el nombre adelante de cada
  línea) y Shell entra a uno Running. Copiar logs enmascara tokens,
  contraseñas y credenciales en URLs (ctrl+clic copia sin redactar).
- **Port-forward**: de un Service o de un pod concreto, en loopback. Con
  alias opcional en `/etc/hosts` (vía pkexec, archivo temporal en
  `XDG_RUNTIME_DIR` 0700) para consumirlo por nombre.
- **Recursos custom**: la tabla muestra las `additionalPrinterColumns` del
  CRD como `kubectl get`; si el CRD no declara ninguna, infiere un Estado
  (phase, conditions…) y un resumen del spec. El detalle muestra además las
  columnas de `-o wide`.
- **Auditoría local**: `acciones.jsonl` (0600) con borrar, escalar,
  reiniciar, aplicar (líneas y hash del manifiesto), shell, logs y
  port-forward, por cluster y con fecha. Se ve en Cluster → Acciones hechas.
- **Modo solo lectura**: `kubo --solo-lectura` (o `KUBO_SOLO_LECTURA=1`)
  quita editar, aplicar, escalar, reiniciar, borrar y shell de toda la UI y
  los frena aunque algo los pida. Para mirar producción sin miedo.
- **Mapa de servicio**: Ingress → Service → workloads → pods del selector,
  dibujado en la pestaña "Mapa" del detalle de un Service.
- **Mapa de configuración del workload**: en Deployments (y StatefulSets,
  DaemonSets, CronJobs, Pods…) la pestaña "Mapa" muestra la estructura
  completa del micro: qué Ingress/Services le mandan tráfico, sus imágenes, y
  qué ConfigMaps, Secrets, PVCs y ServiceAccount referencia (envFrom, env,
  volúmenes, projected, imagePullSecrets) — con ⚠ en rojo si la referencia
  no existe en el namespace. Cada referencia es una cajita clickeable que
  navega a ese recurso (cambia la vista y abre su detalle).
- **Secrets protegidos**: base64 no es cifrado, así que los valores llegan
  enmascarados — tanto en `data` como en la anotación
  `last-applied-configuration`, que guarda el objeto entero. En el Resumen se
  revelan (ya decodificados) clave por clave; el YAML tiene un botón "Revelar"
  que relee el objeto, y `Aplicar` se bloquea mientras esté enmascarado para
  no escribir el marcador como valor.
- **Paleta de comandos** (`Ctrl+K`): busca a la vez entre las vistas del
  sidebar y los recursos del cluster por nombre (Pods, Deployments, Services,
  Ingresses, ConfigMaps, Secrets, StatefulSets, DaemonSets, CronJobs, Jobs,
  PVCs, Nodes, Namespaces, HPAs, ServiceAccounts y los CRDs habituales —Argo
  Application/Rollout, Certificate, HelmRelease, ExternalSecret, NodePool,
  ServiceMonitor, VirtualService, Gateway, HTTPRoute, Workflow— si el cluster
  los sirve). Debounce de 250 ms, ↑↓ para moverse, ↵ para abrir.
- **Paneles múltiples** (hasta 4): varios clusters a la vez, o varios recursos
  del mismo cluster, lado a lado. Las conexiones se comparten por contexto.
- **Conexión rápida**: discovery agregado (2 requests) con fallback al
  recorrido por grupo, y versión/discovery/namespaces en paralelo.

## Compilar

```bash
cargo build --release
./target/release/kubo
```

## Arquitectura

El hilo de UI no hace I/O. Todo el tráfico contra el API server corre en un
runtime de tokio y llega por canal (`flume`); cada evento despierta el
repintado, así que la app está a 0% de CPU cuando el cluster está quieto.

```
src/
  main.rs      arranque de eframe
  app.rs       estado y ciclo de vida de las tareas async
  k8s/         kubeconfig, discovery, watch, logs, detalle
  nav.rs       árbol del sidebar (catálogo fijo × discovery real + operadores)
  columns.rs   qué columnas tiene cada Kind y cómo se extraen
  store.rs     filas en memoria: caché de celdas, orden y filtro
  ui/          topbar, sidebar, tabla, detalle, logs
  theme.rs     paleta
```

Cada vista lleva un `token`. Al cambiar de recurso el watch viejo se aborta y
lo que llegue tarde se descarta: cambiar rápido de pantalla nunca mezcla filas
de dos recursos distintos.

## Harness de depuración

Variables de entorno para probar sin clickear (viven en
`src/app/pruebas.rs`; inertes si no están definidas, y solo actúan sobre la
sesión propia — no mutan nada por sí solas, salvo `KUBO_TEST_PF`, que levanta
un túnel local): `KUBO_TEST_SHELL=ns/pod` (abre la shell al cargar;
`KUBO_TEST_SHELL_CMD` manda un comando), `KUBO_TEST_MAPA=ns/service` (abre el
mapa), `KUBO_TEST_WMAPA=ns/deployment` (mapa de configuración),
`KUBO_TEST_IRA=Kind:ns:name` (navegación, con `KUBO_TEST_TAB=Yaml|Eventos|Mapa`),
`KUBO_TEST_WL=logs|shell:ns/name` (logs o shell de un workload),
`KUBO_TEST_PF=ns/service-o-pod` (port-forward con valores por defecto),
`KUBO_TEST_SEL=ns/a,ns/b` (marca filas), `KUBO_TEST_CONFIRM=[escalar:|aplicar:]Kind:ns:nombre[,nombre]`
(abre el modal sin ejecutar), `KUBO_TEST_PROD=1` (trata el contexto como
producción), `KUBO_TEST_VISTA=auditoria|forwards`, `KUBO_TEST_PALETTE=texto`,
`KUBO_TEST_PANES=n`, `KUBO_TEST_NAV=0`, `KUBO_TEST_SIZE=1400x1000`.

Para capturas de pantalla reproducibles sin tocar el escritorio, kubo corre
igual dentro de un `sway` headless (`WLR_BACKENDS=headless`) y se captura con
`grim -o HEADLESS-1`.

## Todavía no

- Drag para reordenar paneles.
- Favoritos de recursos concretos (hoy solo de vistas).
- Tema claro.

## Instalar en Linux

```sh
cargo build --release
./instalar.sh
```

Deja el binario en `~/.local/bin`, el ícono en el tema hicolor y la entrada
en el launcher. La entrada usa la ruta absoluta `~/.local/bin/kubo`, así no
depende del `PATH` de la sesión gráfica. Sin sudo. Para sacarlo:
`./instalar.sh --quitar`.

El `.tar.gz` de cada release Linux ya incluye `kubo`, `instalar.sh` y los
iconos. Después de extraerlo, ejecutá `./instalar.sh`.

## Publicar una versión

```sh
./publicar.sh v0.2.0 "qué cambió"
```

El push del tag dispara el workflow, que compila los tres sistemas, ejecuta
cada binario en su plataforma y publica la release. `dist.sh` compila el de
Linux localmente en un contenedor con glibc viejo, por si querés uno sin
pasar por CI.

## Licencia

MIT o Apache-2.0, a elección — la convención del ecosistema Rust. Ver
[LICENSE-MIT](LICENSE-MIT) y [LICENSE-APACHE](LICENSE-APACHE).
