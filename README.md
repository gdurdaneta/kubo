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
  siempre con modal de confirmación. Aplicar rechaza cambios de nombre o
  namespace: en Kubernetes los recursos no se renombran, y aplicar sobre otro
  nombre sería tocar un objeto distinto del que se está mirando.
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
  PVCs, Nodes). Debounce de 250 ms, ↑↓ para moverse, ↵ para abrir.
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

Variables de entorno para probar sin clickear: `KUBO_TEST_SHELL=ns/pod`
(abre la shell al cargar; `KUBO_TEST_SHELL_CMD` manda un comando),
`KUBO_TEST_MAPA=ns/service` (abre el mapa), `KUBO_TEST_WMAPA=ns/deployment`
(mapa de configuración), `KUBO_TEST_IRA=Kind:ns:name` (navegación, con
`KUBO_TEST_TAB=Yaml|Eventos|Mapa`), `KUBO_TEST_PALETTE=texto`,
`KUBO_TEST_PANES=n`.

## Todavía no

- Métricas de CPU/memoria vía metrics-server.
- `port-forward`.
- Drag para reordenar paneles; layouts guardados.
