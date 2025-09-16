# Server API Tests

This directory contains JavaScript tests for the app-scaffold server API endpoints using the Jest testing framework.

## Prerequisites

- Node.js 18+ (for native fetch support)
- The server must be running before executing tests

## Setup

1. Install dependencies:
   ```bash
   cd tests
   npm install
   ```

2. Make sure your server is running on the expected port (default: `http://localhost:3000`)

## Running Tests

### Run All Tests
```bash
npm test
```

### Run Tests with Different Options
```bash
# Run tests in watch mode (re-runs on file changes)
npm run test:watch

# Run tests with coverage report
npm run test:coverage

# Run tests with verbose output
npm run test:verbose
```

### Run Individual Test Suites
```bash
# Test health endpoint only
npm run test:health

# Test config endpoint only
npm run test:config

# Test increment endpoint only
npm run test:increment

# Test CORS headers only
npm run test:cors

# Test error handling only
npm run test:error

# Test performance only
npm run test:performance
```

### Run Tests with Custom Server URL
```bash
API_BASE_URL=http://localhost:8080 npm test
```

## Test Coverage

The Jest test suite is organized into logical test suites covering the following areas:

### 1. Health Endpoint (`GET /_health`)
- ✅ Returns OK status (200)
- ✅ Responds quickly (< 1 second)
- ✅ Server connectivity check in `beforeAll`

### 2. Config Endpoint (`GET /api/config`)
- ✅ Returns configuration data with contract name
- ✅ Returns valid JSON response
- ✅ Contract name is a non-empty string

### 3. Increment Endpoint (`POST /api/increment`)
- ✅ Rejects requests without authentication (401)
- ✅ Accepts requests with proper `x-user` header
- ✅ Requires valid JSON body
- ✅ Requires `wallet_blobs` in request body
- ✅ Handles mock data gracefully (200 or 400 expected)

### 4. CORS Headers
- ✅ Includes CORS headers in responses
- ✅ Handles preflight OPTIONS requests properly

### 5. Error Handling
- ✅ Returns 404 for non-existent endpoints
- ✅ Handles malformed JSON requests gracefully

### 6. Performance
- ✅ All endpoints respond within reasonable time (< 2 seconds)

## Configuration

### Server URL
By default, tests expect the server to be running on `http://localhost:3000`. You can change this by:

1. Setting an environment variable:
   ```bash
   API_BASE_URL=http://your-server-url:port npm test
   ```

2. Or modifying the `BASE_URL` constant in `server-api.test.js`:
   ```javascript
   const BASE_URL = process.env.API_BASE_URL || 'http://your-server-url:port';
   ```

### Test Data
The tests use mock wallet blobs. You may need to adjust these in `server-api.test.js` based on your actual blob format:

```javascript
const MOCK_WALLET_BLOBS = [
    "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    "0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321"
];
```

### Test User
The tests use a default test user. You can modify this in `server-api.test.js`:

```javascript
const TEST_USER = "your-test-user";
```

## Expected Results

All tests should pass when the server is running properly:

- **Health endpoint**: Returns 200 with "OK" and responds quickly
- **Config endpoint**: Returns 200 with valid contract configuration
- **Increment without auth**: Returns 401 (Unauthorized)
- **Increment with auth**: Returns 200 (success) or 400 (bad request due to mock data)
- **CORS**: Includes proper CORS headers
- **Error handling**: Proper 404 responses for invalid endpoints
- **Performance**: All endpoints respond within 2 seconds

## Troubleshooting

### Server Not Running
If you get connection errors, make sure your server is running:
```bash
# From the project root
cargo run --bin server
```

### Port Issues
If your server runs on a different port, update the `BASE_URL` in the test file.

### Authentication Issues
Make sure your server is configured to accept the test user. The tests use the `x-user` header for authentication.

### Mock Data Issues
The increment test uses mock wallet blobs that may not be valid for your system. This is expected - the test verifies that authentication works, even if the transaction fails due to invalid data.

## Adding New Tests

To add new tests using Jest:

1. Add a new `describe` block or `test` within an existing `describe` block in `server-api.test.js`
2. Optionally add a new npm script in `package.json` for running specific test suites

Example:
```javascript
describe('New Endpoint', () => {
    test('should return expected data', async () => {
        const response = await makeRequest(`${BASE_URL}/api/new-endpoint`);
        
        expect(response.status).toBe(200);
        expect(response.data).toHaveProperty('expected_field');
    });

    test('should handle errors gracefully', async () => {
        const response = await makeRequest(`${BASE_URL}/api/new-endpoint/invalid`);
        
        expect(response.status).toBe(404);
    });
});
```

### Jest Features Used

- **`describe`**: Groups related tests together
- **`test`**: Individual test cases
- **`beforeAll`**: Setup that runs once before all tests
- **`expect`**: Jest assertions for testing values
- **`toHaveProperty`**: Check if object has specific property
- **`toBe`**: Exact equality assertion
- **`toBeLessThan`**: Numeric comparison
- **`toContain`**: Array/string contains assertion
