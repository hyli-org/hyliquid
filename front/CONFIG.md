# Runtime Configuration

The frontend application supports runtime configuration through environment variables in Docker, allowing you to configure the application without rebuilding the image.

## Available Configuration Options

- `API_BASE_URL`: Base URL for the API (default: `http://localhost:3000`)
- `BACKEND_API_URL`: Backend API URL (default: `http://localhost:9002`)

## Development

For local development, configuration is set in `public/config.js` or via Vite environment variables (`.env` file):

```bash
# .env file
VITE_API_BASE_URL=http://localhost:3000
VITE_BACKEND_API_URL=http://localhost:9002
```

Then run:

```bash
bun run dev
```

## Docker (Production)

When running in Docker, pass environment variables at **runtime** (not build time):

### Using docker run

```bash
docker run -p 80:80 \
  -e API_BASE_URL=https://api.example.com \
  -e BACKEND_API_URL=https://backend.example.com \
  your-image-name
```

### Using docker-compose

```yaml
services:
    frontend:
        image: your-image-name
        ports:
            - "80:80"
        environment:
            - API_BASE_URL=https://api.example.com
            - BACKEND_API_URL=https://backend.example.com
```

### Using Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
    name: frontend
spec:
    template:
        spec:
            containers:
                - name: frontend
                  image: your-image-name
                  ports:
                      - containerPort: 80
                  env:
                      - name: API_BASE_URL
                        value: "https://api.example.com"
                      - name: BACKEND_API_URL
                        value: "https://backend.example.com"
```

## How It Works

1. **Build time**: The application is built once with no hardcoded URLs
2. **Container startup**: The `docker-entrypoint.sh` script generates `config.js` from environment variables
3. **Runtime**: The browser loads `config.js` before the application, making the configuration available via `window.__CONFIG__`
4. **Application**: `src/config.ts` reads from `window.__CONFIG__` (production) or falls back to Vite env vars (development)

This approach allows you to deploy the same Docker image across different environments (staging, production, etc.) with different configurations.
