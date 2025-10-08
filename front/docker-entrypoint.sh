#!/bin/sh
set -e

# Generate config.js with environment variables at runtime
cat > /usr/share/nginx/html/config.js << EOF
// Runtime configuration - generated from environment variables
window.__CONFIG__ = {
  API_BASE_URL: "${API_BASE_URL:-http://localhost:3000}",
  BACKEND_API_URL: "${BACKEND_API_URL:-http://localhost:9002}",
  WEBSOCKET_URL: "${WEBSOCKET_URL:-ws://localhost:3000/ws}"
};
EOF

echo "Generated config.js with:"
echo "  API_BASE_URL: ${API_BASE_URL:-http://localhost:3000}"
echo "  BACKEND_API_URL: ${BACKEND_API_URL:-http://localhost:9002}"
echo "  WEBSOCKET_URL: ${WEBSOCKET_URL:-ws://localhost:3000/ws}"

# Execute the main container command (nginx)
exec "$@"
