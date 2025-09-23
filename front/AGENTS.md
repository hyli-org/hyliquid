# Repository Guidelines

## Project Structure & Module Organization

- `src/`: Vue 3 + TypeScript source. Start with `main.ts`, root `App.vue`.
- `src/assets/`: App-specific assets; `public/` for static files served as-is.
- Suggested structure: reusable generic components in `src/components`, rest follows feature/domain (e.g., `src/auth/`, `src/dashboard/`).
- No stores, we use basic reactive objects in ts files and composables. Keep it simple.
- Routing: defined in `src/main.ts` (Vue Router). Add routes via a dedicated `src/routes.ts` when it expands.

## Coding Style & Naming Conventions

- All as expected from a vite / vue3 / typescript project.
- Tailwind should be used for styling. Avoid dedicated CSS files unless absolutely necessary.
