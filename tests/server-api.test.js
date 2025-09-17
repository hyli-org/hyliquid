/**
 * Server API Tests using Jest
 * 
 * This file contains tests for the server API endpoints using Jest framework.
 * Make sure the server is running before executing these tests.
 * 
 * To run: npm test
 */

// env var provided by hylix testing environment
const BASE_URL = process.env.API_BASE_URL || 'http://localhost:4002';
const NODE_BASE_URL = process.env.HYLI_NODE_BASE_URL || 'http://localhost:4321';
const HYLI_WALLET_API_URL = process.env.HYLI_WALLET_API_URL || 'http://localhost:4000';

import { verifyIdentity, IndexerService } from "hyli-wallet";
import { build_blob } from "hyli-check-secret";

const walletIndexer = IndexerService.initialize(HYLI_WALLET_API_URL);

const bob = await walletIndexer.getAccountInfo("bob");

const TEST_USER = "bob";
const TEST_IDENTITY = `bob@wallet`;
const TEST_PASSWORD = `hylisecure:${bob.salt}`;
const WALLET_BLOBS = [verifyIdentity(TEST_USER, bob.nonce + 1), await build_blob(TEST_IDENTITY, TEST_PASSWORD)];

/**
 * Helper function to make HTTP requests
 */
async function makeRequest(url, options = {}) {
  const response = await fetch(url, {
    headers: {
      'Content-Type': 'application/json',
      ...options.headers
    },
    ...options
  });

  const data = await response.text();
  let jsonData;
  try {
    jsonData = JSON.parse(data);
  } catch (e) {
    jsonData = data;
  }

  return {
    status: response.status,
    statusText: response.statusText,
    data: jsonData,
    headers: response.headers
  };
}

describe('Server API Tests', () => {
  // Global setup - check if server is reachable
  beforeAll(async () => {
    try {
      const response = await fetch(`${BASE_URL}/_health`, {
        method: 'GET',
        signal: AbortSignal.timeout(5000) // 5 second timeout
      });
      if (!response.ok) {
        throw new Error(`Server not responding: ${response.status}`);
      }
    } catch (error) {
      throw new Error(`Cannot connect to server at ${BASE_URL}. Make sure the server is running. Error: ${error.message}`);
    }
  });

  describe('Health Endpoint', () => {
    test('should return OK status', async () => {
      const response = await makeRequest(`${BASE_URL}/_health`);

      expect(response.status).toBe(200);
      expect(response.data).toBe("OK");
    });

    test('should respond quickly', async () => {
      const start = Date.now();
      await makeRequest(`${BASE_URL}/_health`);
      const duration = Date.now() - start;

      expect(duration).toBeLessThan(1000); // Should respond within 1 second
    });
  });

  describe('Config Endpoint', () => {
    test('should return configuration data', async () => {
      const response = await makeRequest(`${BASE_URL}/api/config`);

      expect(response.status).toBe(200);
      expect(response.data).toHaveProperty('contract_name');
      expect(typeof response.data.contract_name).toBe('string');
      expect(response.data.contract_name.length).toBeGreaterThan(0);
    });

    test('should return valid JSON', async () => {
      const response = await makeRequest(`${BASE_URL}/api/config`);

      expect(response.status).toBe(200);
      expect(typeof response.data).toBe('object');
      expect(response.data).not.toBeNull();
    });
  });

  describe('Orderbook Operations', () => {
    const ORDER_ID = `order_${Date.now()}`;
    const PAIR_TOKEN1 = 'ETH';
    const PAIR_TOKEN2 = 'USDC';
    const QUANTITY = 100;
    const PRICE = 2000;
    const TOKEN = 'ETH';
    const AMOUNT = 50;

    describe('Create Order', () => {
      test('should create a buy order with price', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/create-order`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            order_id: `${ORDER_ID}_buy`,
            order_type: 'buy',
            price: PRICE,
            pair_token1: PAIR_TOKEN1,
            pair_token2: PAIR_TOKEN2,
            quantity: QUANTITY
          })
        });

        console.log("Create buy order response:", response);
        expect([200, 201]).toContain(response.status);
      });

      test('should create a sell order with price', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/create-order`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            order_id: `${ORDER_ID}_sell`,
            order_type: 'sell',
            price: PRICE,
            pair_token1: PAIR_TOKEN1,
            pair_token2: PAIR_TOKEN2,
            quantity: QUANTITY
          })
        });

        console.log("Create sell order response:", response);
        expect([200, 201]).toContain(response.status);
      });

      test('should create a market order without price', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/create-order`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            order_id: `${ORDER_ID}_market`,
            order_type: 'buy',
            pair_token1: PAIR_TOKEN1,
            pair_token2: PAIR_TOKEN2,
            quantity: QUANTITY
          })
        });

        console.log("Create market order response:", response);
        expect([200, 201]).toContain(response.status);
      });

      test('should reject order with invalid order type', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/create-order`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            order_id: `${ORDER_ID}_invalid`,
            order_type: 'invalid_type',
            price: PRICE,
            pair_token1: PAIR_TOKEN1,
            pair_token2: PAIR_TOKEN2,
            quantity: QUANTITY
          })
        });

        expect(response.status).toBe(400);
      });

      test('should reject order with missing required fields', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/create-order`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            order_type: 'buy',
            quantity: QUANTITY
            // Missing order_id, pair_token1, pair_token2
          })
        });

        expect(response.status).toBe(400);
      });
    });

    describe('Cancel Order', () => {
      test('should cancel an existing order', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/cancel`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            order_id: `${ORDER_ID}_buy`
          })
        });

        console.log("Cancel order response:", response);
        expect([200, 204]).toContain(response.status);
      });

      test('should handle cancellation of non-existent order', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/cancel`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            order_id: 'non_existent_order'
          })
        });

        // Should either succeed (idempotent) or return 404
        expect([200, 204, 404]).toContain(response.status);
      });

      test('should reject cancel request with missing order_id', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/cancel`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS
            // Missing order_id
          })
        });

        expect(response.status).toBe(400);
      });
    });

    describe('Deposit Tokens', () => {
      test('should deposit tokens successfully', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/deposit`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: TOKEN,
            amount: AMOUNT
          })
        });

        console.log("Deposit response:", response);
        expect([200, 201]).toContain(response.status);
      });

      test('should handle deposit with zero amount', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/deposit`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: TOKEN,
            amount: 0
          })
        });

        // Zero amount should be rejected
        expect(response.status).toBe(400);
      });

      test('should reject deposit with missing fields', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/deposit`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: TOKEN
            // Missing amount
          })
        });

        expect(response.status).toBe(400);
      });

      test('should handle deposit with invalid token', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/deposit`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: '',
            amount: AMOUNT
          })
        });

        expect(response.status).toBe(400);
      });
    });

    describe('Withdraw Tokens', () => {
      test('should withdraw tokens successfully', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/withdraw`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: TOKEN,
            amount: AMOUNT
          })
        });

        console.log("Withdraw response:", response);
        expect([200, 204]).toContain(response.status);
      });

      test('should handle withdrawal with zero amount', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/withdraw`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: TOKEN,
            amount: 0
          })
        });

        // Zero amount should be rejected
        expect(response.status).toBe(400);
      });

      test('should handle withdrawal exceeding balance', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/withdraw`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: TOKEN,
            amount: 999999999 // Very large amount
          })
        });

        // Should reject insufficient balance
        expect([400, 422]).toContain(response.status);
      });

      test('should reject withdrawal with missing fields', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/withdraw`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            amount: AMOUNT
            // Missing token
          })
        });

        expect(response.status).toBe(400);
      });
    });

    describe('Authentication and Authorization', () => {
      test('should reject requests without wallet_blobs', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/deposit`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            token: TOKEN,
            amount: AMOUNT
            // Missing wallet_blobs
          })
        });

        expect([400, 401, 403]).toContain(response.status);
      });

      test('should reject requests without x-user header', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/deposit`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
            // Missing x-user header
          },
          body: JSON.stringify({
            wallet_blobs: WALLET_BLOBS,
            token: TOKEN,
            amount: AMOUNT
          })
        });

        expect([400, 401, 403]).toContain(response.status);
      });

      test('should reject requests with invalid wallet_blobs', async () => {
        const response = await makeRequest(`${BASE_URL}/api/orderbook/deposit`, {
          method: 'POST',
          headers: {
            'x-user': TEST_USER,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            wallet_blobs: ['invalid_blob_1', 'invalid_blob_2'],
            token: TOKEN,
            amount: AMOUNT
          })
        });

        expect([400, 401, 403]).toContain(response.status);
      });
    });
  });

  describe('CORS Headers', () => {
    test('should include CORS headers in responses', async () => {
      const response = await makeRequest(`${BASE_URL}/_health`);

      const corsHeaders = {
        'access-control-allow-origin': response.headers.get('access-control-allow-origin'),
        'access-control-allow-methods': response.headers.get('access-control-allow-methods'),
        'access-control-allow-headers': response.headers.get('access-control-allow-headers')
      };

      expect(corsHeaders['access-control-allow-origin']).toBeTruthy();
    });

    test('should handle preflight OPTIONS requests', async () => {
      const response = await makeRequest(`${BASE_URL}/_health`, {
        method: 'OPTIONS',
        headers: {
          'Origin': 'http://localhost:3001',
          'Access-Control-Request-Method': 'GET'
        }
      });

      // OPTIONS request should not return an error
      expect(response.status).toBeLessThan(500);
    });
  });

  describe('Error Handling', () => {
    test('should return 404 for non-existent endpoints', async () => {
      const response = await makeRequest(`${BASE_URL}/api/nonexistent`);

      expect(response.status).toBe(404);
    });

    test('should handle malformed requests gracefully', async () => {
      const response = await makeRequest(`${BASE_URL}/api/increment`, {
        method: 'POST',
        headers: {
          'x-user': TEST_USER,
          'Content-Type': 'application/json'
        },
        body: '{"invalid": json}'
      });

      expect(response.status).toBe(400);
    });
  });

});
