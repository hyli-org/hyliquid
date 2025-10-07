# AGENTS.md — Guides d’agents IA pour un backend Rust

> **But du fichier**  
> Servir de *mémoire partagée* entre toi et l’IA (Cursor) pour accélérer le développement d’un backend Rust robuste.  
> **Comment l’utiliser dans Cursor ?** Ouvre ce fichier et **ajoute-le au contexte** du chat de Cursor (option “Add files to chat” / “Include current file”). Dans tes requêtes, **réfère-toi explicitement au nom d’un agent** ci‑dessous (ex. *« Suis l’agent TESTS pour générer des tests unitaires »*).

---

## 0) Snapshot du projet (à renseigner rapidement)
- **Nom du crate** : `{{CRATE_NAME}}`
- **Framework HTTP** : `axum` | `actix-web` | `warp`
- **RPC** : `tonic` (gRPC) | `none`
- **DB** : `postgres` via `sqlx` | `sea-orm` | `diesel`
- **Migrations** : `sqlx migrate` | `refinery` | `barrel`
- **Queue/stream** : `kafka` | `nats` | `rabbitmq` | `none`
- **Config** : `serde` + `figment`/`config`
- **Observabilité** : `tracing` + `tracing-subscriber` + `opentelemetry`
- **Tests** : `proptest` | `insta` | tests d’intégration
- **Erreurs** : `thiserror` (lib) | `anyhow` (bin)
- **Lint/format** : `clippy` + `rustfmt`
- **Build** : `cargo` (+ `cross`/`nix` si besoin)
- **Cibles** : `x86_64-unknown-linux-gnu` | autre
- **Secrets** : `.env` via `dotenvy` | `AWS Secrets Manager` | `sops`

> Mets ce bloc à jour. L’IA s’appuiera dessus pour choisir les bons patterns/CRATES.

---

## 1) Garde‑fous (appliqués par tous les agents)

- **Async** : ne jamais `await` en tenant un `MutexGuard`. Préférer `RwLock` ou refactorer pour éviter les verrous larges.  
- **CPU‑bound** : utiliser `tokio::task::spawn_blocking` pour le CPU lourd.  
- **Erreurs** : pas de `unwrap()`/`expect()` dans le code prod ; utiliser `thiserror`/`anyhow` + `?`.  
- **I/O & streaming** : éviter de charger en mémoire entière ; préférer `Stream`, `Bytes`, back‑pressure.  
- **Observabilité** : instrumenter `tracing` (span par requête, IDs corrélés).  
- **Sécurité** : valider les entrées, gérer les secrets via le provider choisi, nettoyer logs (pas de PII).  
- **DB** : transactions quand nécessaire, timeouts, pool configuré, requêtes préparées (`sqlx` en mode offline si possible).  
- **API** : versions d’API, schémas stables (OpenAPI/Protobuf), erreurs JSON uniformes.  
- **Tests** : tests rapides, isolés, reproductibles ; fuzz/property‑based pour fonctions critiques.  
- **Docs** : toute feature ajoutée → section README + CHANGELOG.

---

## 2) Rôles d’agents

Chaque agent décrit : **Objectif**, **Quand l’utiliser**, **Entrées attendues**, **Étapes**, **Livrables**, **Critères d’acceptation**.

### AGENT ARCHITECTE
- **Objectif** : cadrer une feature, définir les limites (HTTP/RPC, DB, domaines), éviter la dette.  
- **Quand** : avant d’implémenter une nouvelle fonctionnalité.  
- **Entrées** : description métier, contraintes perf/sécu, snapshot projet (section 0).  
- **Étapes** :  
  1) Clarifier inputs/outputs métier, invariants, erreurs.  
  2) Proposer design : handlers, services, repos, schémas DB, events.  
  3) Lister risques + stratégies de mitigation.  
  4) Donner un plan d’implémentation **par commits**.  
- **Livrables** : markdown “Design Doc” + todo technique.  
- **Critères** : design cohérent, testable, rétro‑compatible, mesurable (observabilité).

### AGENT API HTTP
- **Objectif** : concevoir/implémenter endpoints REST (Axum/Actix).  
- **Entrées** : modèles, routes, erreurs, schéma OpenAPI si présent.  
- **Étapes** :  
  1) Définir routes, DTO (`serde`), validations.  
  2) Implémenter handlers, extractions (`Json`, `Path`, `State`).  
  3) Erreurs uniformes (type `ApiError`).  
  4) Ajouter `tracing` + métriques.  
- **Livrables** : code Rust + exemple de requêtes `curl` + mise à jour OpenAPI.  
- **Critères** : stateless, idempotence respectée (quand applicable), couverture de tests.

### AGENT gRPC
- **Objectif** : services RPC via `tonic`.  
- **Entrées** : `.proto`, contrats, perfs attendues.  
- **Étapes** : générer stubs, implémenter services, erreurs mappées, intercepteurs (auth/log).  
- **Livrables** : services + fichiers `.proto` + guide d’usage client.  
- **Critères** : rétro‑compat protocolaire, timeouts, backoff côté client.

### AGENT BASE DE DONNÉES
- **Objectif** : schémas, migrations, accès DB (`sqlx`/`sea-orm`/`diesel`).  
- **Entrées** : besoins de persistance, contraintes d’intégrité.  
- **Étapes** :  
  1) Proposer schéma (clé primaire, index, FKs).  
  2) Écrire migrations, prévoir rollbacks.  
  3) Implémenter DAO/Repo, transactions.  
  4) Tests d’intégration DB.  
- **Livrables** : migrations + code d’accès + seeds si nécessaire.  
- **Critères** : normalisation raisonnable, perfs ok, migrations sûres.

### AGENT SÉCURITÉ
- **Objectif** : revue sécurité et conformité.  
- **Étapes** : checklist authN/authZ, secrets, validation, journaux, dépendances (audit), headers HTTP, CORS.  
- **Livrables** : rapport risques + patchs proposés.  
- **Critères** : risques classés (H/M/B), plans d’atténuation et tests.

### AGENT PERFORMANCE
- **Objectif** : débusquer contentions/allocations inutiles ; conseiller profils.  
- **Étapes** : instrumentation `tracing`, propositions de benchmarks (`criterion`), éviter clones, `Arc` placement, pool tuning.  
- **Livrables** : plan d’optimisation + diffs de code.  
- **Critères** : latence/throughput améliorés sans complexifier à l’excès.

### AGENT TESTS
- **Objectif** : générer/renforcer tests unitaires & intégration.  
- **Étapes** : cas nominaux/erreurs, tests property‑based (`proptest`) pour fonctions pures, fixtures DB, tests de contrat API.  
- **Livrables** : fichiers de tests + scripts `cargo test`.  
- **Critères** : couverture accrue sur chemins critiques, flakiness faible.

### AGENT OBSERVABILITÉ
- **Objectif** : logs structurés, traces, métriques.  
- **Étapes** : intégrer `tracing`, niveaux log, spans par requête, exporter OTLP (optionnel), dashboards de base.  
- **Livrables** : code d’instrumentation + doc “comment diagnostiquer”.  
- **Critères** : corrélation facile, bruit réduit, coûts maîtrisés.

### AGENT DEVOPS/CI
- **Objectif** : pipelines reproductibles, images légères.  
- **Étapes** : Dockerfile multi‑étapes, cache dep, tests en CI, lint clippy/rustfmt, build release, scan vulnérabilités.  
- **Livrables** : fichiers CI/CD + Dockerfile + doc déploiement.  
- **Critères** : pipeline < ~10 min, artefacts versionnés, provenance claire.

### AGENT DOCS
- **Objectif** : README, CHANGELOG, guides d’intégration.  
- **Étapes** : générer MD concis, exemples `curl`, diagrammes simples (ASCII), conventions commit.  
- **Livrables** : docs + PR de mise à jour.  
- **Critères** : complet, à jour, actionnable.

---

## 3) Bibliothèque de prompts (copier‑coller dans Cursor)

> **Astuce** : commence par *« Utilise AGENTS.md et joue le rôle AGENT X »* pour forcer l’alignement.

### 3.1 Architecte
**Prompt :**
```
Utilise le rôle **AGENT ARCHITECTE** défini dans AGENTS.md. 
Contexte : {{description métier et contraintes}}.
Objectif : produire un mini design doc (handlers, services, repos, schéma DB, events), risques + mitigations, et un plan d’implémentation en 5–8 commits.
Respecte les garde‑fous de la section 1.
```

### 3.2 API HTTP (Axum)
**Prompt :**
```
Rôle : **AGENT API HTTP** (AGENTS.md). 
Crée les routes et handlers Axum pour {{ressource}} avec DTO sérialisés/validés, erreurs uniformes, et instrumentation `tracing`.
Ajoute des exemples `curl` et mets à jour l’OpenAPI si présent.
```

**Exemple handler (schéma) :**
```rust
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Create{{Entity}}Request { /* champs */ }

#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub async fn create_{{resource}}(
    State(app): State<AppState>,
    Json(req): Json<Create{{Entity}}Request>,
) -> Result<Json<{{Entity}}>, ApiError> {
    // TODO: logique, validations
    Ok(Json({{Entity}}{/*...*/}))
}
```

### 3.3 Base de données (SQLx)
**Prompt :**
```
Rôle : **AGENT BASE DE DONNÉES**. 
Propose le schéma et les migrations SQLx pour {{ressource}} (Postgres), avec index pertinents et clés étrangères. 
Écris le repo (fonctions CRUD) avec transactions quand nécessaire. Fournis des tests d’intégration.
```

### 3.4 Sécurité
**Prompt :**
```
Rôle : **AGENT SÉCURITÉ**. 
Passe en revue {{fichier/feature}}. Liste risques (H/M/B) concernant validation d’entrée, authN/authZ, secrets, logs, en-têtes HTTP, CORS, dépendances. Propose correctifs concrets (diffs de code).
```

### 3.5 Performance
**Prompt :**
```
Rôle : **AGENT PERFORMANCE**. 
Analyse {{fichier}}. Identifie allocations excessives, clones inutiles, blocages async. Propose instrumentation `tracing`, benchmarks `criterion`, et un plan d’optimisation avec diffs.
```

### 3.6 Tests
**Prompt :**
```
Rôle : **AGENT TESTS**. 
Génère tests unitaires et d’intégration pour {{module}}. Couvre cas nominaux/erreurs. Ajoute un exemple `proptest` pour une fonction pure critique.
```

### 3.7 Observabilité
**Prompt :**
```
Rôle : **AGENT OBSERVABILITÉ**.
Ajoute `tracing` (spans/fields) à {{handlers/services}}. Fournis un init propre du logger, niveaux par défaut, et un guide “comment diagnostiquer une requête lente”.
```

### 3.8 DevOps/CI
**Prompt :**
```
Rôle : **AGENT DEVOPS/CI**.
Écris un Dockerfile multi‑étapes minimal pour le binaire {{BIN}} (base distroless/ubi), plus une config CI (lint + tests + build). Optimise la taille et le cache.
```

### 3.9 Docs
**Prompt :**
```
Rôle : **AGENT DOCS**.
Mets à jour README avec installation, variables d’env, commandes `cargo`, exemples `curl`, et une section dépannage. Rédige aussi une entrée CHANGELOG.
```

---

## 4) Commandes Cursor (à créer dans “Custom Commands”)

> Indique **Nom** et **Instruction**. Active “Use project context” si disponible.

- **Nom** : 🧭 Architecte — cadrer une feature  
  **Instruction** : *Utilise le rôle AGENT ARCHITECTE d’AGENTS.md. …* (reprends le prompt 3.1)

- **Nom** : 🌐 API — scaffolder endpoints Axum  
  **Instruction** : *Rôle AGENT API HTTP…* (reprends 3.2)

- **Nom** : 🗄️ DB — schéma + migrations SQLx  
  **Instruction** : *Rôle AGENT BASE DE DONNÉES…* (reprends 3.3)

- **Nom** : 🧪 Tests — unitaires & intégration  
  **Instruction** : *Rôle AGENT TESTS…* (reprends 3.6)

- **Nom** : 🔍 Sécu — revue éclair  
  **Instruction** : *Rôle AGENT SÉCURITÉ…* (reprends 3.4)

- **Nom** : 📈 Perf — audit ciblé  
  **Instruction** : *Rôle AGENT PERFORMANCE…* (reprends 3.5)

- **Nom** : 👁 Observabilité — instrumentation  
  **Instruction** : *Rôle AGENT OBSERVABILITÉ…* (reprends 3.7)

- **Nom** : ⚙️ DevOps — Docker + CI minimal  
  **Instruction** : *Rôle AGENT DEVOPS/CI…* (reprends 3.8)

- **Nom** : 📚 Docs — README + Changelog  
  **Instruction** : *Rôle AGENT DOCS…* (reprends 3.9)

---

## 5) Extraits & gabarits utiles

### 5.1 Initialisation `tracing`
```rust
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=debug"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .init();
}
```

### 5.2 Erreurs uniformes (API)
```rust
#[derive(Debug, serde::Serialize)]
struct ErrorBody { code: &'static str, message: String }

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, ErrorBody { code: "not_found", message: "Ressource introuvable".into() }),
            ApiError::Validation(m) => (StatusCode::UNPROCESSABLE_ENTITY, ErrorBody { code: "validation", message: m }),
            ApiError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, ErrorBody { code: "internal", message: format!("{e}") }),
        };
        (status, Json(body)).into_response()
    }
}
```

### 5.3 Dockerfile multi‑étapes (exemple)
```dockerfile
# build
FROM rust:1.81 as build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

# runtime (distroless ou gcr.io/distroless/cc)
FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=build /app/target/release/{{BIN_NAME}} /app/{{BIN_NAME}}
USER nonroot:nonroot
CMD ["/app/{{BIN_NAME}}"]
```

### 5.4 Checklists rapides

**Sécurité**
- [ ] Valider toutes les entrées (tailles, formats).  
- [ ] AuthN/AuthZ explicites (rôles/scopes).  
- [ ] Secrets hors repo ; rotation prévue.  
- [ ] Pas de PII en logs.  
- [ ] Headers de sécurité + CORS minimal.

**Performance**
- [ ] Pas de `clone()` superflu dans les hot paths.  
- [ ] Pas de blocage sync en async.  
- [ ] Pool DB dimensionné (connexions + timeouts).  
- [ ] Benchmarks pour fonctions sensibles.  
- [ ] Traces sur endpoints critiques.

**Fiabilité**
- [ ] Retries avec backoff là où nécessaire.  
- [ ] Idempotence des opérations critiques.  
- [ ] Tests d’intégration DB/API.  
- [ ] Migrations avec rollback testé.  
- [ ] Alerting sur erreurs fatales.

---

## 6) Qualité : commandes utiles

```bash
# Format & lint
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings

# Tests + couverture (ex: cargo-llvm-cov)
cargo test
# cargo llvm-cov --ignore-filename-regex '(.*/tests/.*|.*/examples/.*)' --html
```

---

## 7) Gérer le changement

- **Conventional Commits** : `feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `test:`, `chore:`.  
- **PR template** (à mettre dans `.github/pull_request_template.md`) :

```md
## Objet
- …

## Changements clés
- …

## Tests
- [ ] Unitaires
- [ ] Intégration
- [ ] Manuels (instructions)

## Impacts & risques
- …

## Observabilité
- [ ] Traces/Logs ajoutés
- [ ] Métriques/alertes

## Checklist
- [ ] Lint/format ok
- [ ] Migrations testées
- [ ] README/CHANGELOG mis à jour
```
---

### Remarques finales
- Si tu ajoutes de nouvelles règles d’équipe (logs, sécu, perfs), **mets‑les dans la section 1**.  
- Quand tu demandes quelque chose à l’IA, **nomme l’agent** et **réfère ce fichier** pour l’orienter.
