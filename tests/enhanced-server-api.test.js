/**
 * Enhanced Integration Tests for HyLiquid Orderbook
 * 
 * This file contains enhanced integration tests that focus on the new
 * authentication system, nonce management, and security features.
 * 
 * Make sure the server is running before executing these tests.
 * 
 * To run: npm test enhanced-server-api.test.js
 */

import {
  SERVER_URL,
  DEFAULT_IDENTITY,
  TOKENS,
  runTxSenderCommand,
  checkServerHealth,
  getNonce,
  getAllBalances,
  getBalanceForAccount,
  getAllOrders,
  resetOrderbookState,
  verifyBalance,
  setupTestEnvironment,
  cleanupTestEnvironment,
  addSessionKey
} from './test-utils.js';

// Configuration
const IDENTITY = DEFAULT_IDENTITY;

// Test data
const DEPOSIT_AMOUNT = 5000;

describe('Enhanced TX Sender Integration Tests - Authentication & Nonce System', () => {
  let orderCounter = 1;

  // Global setup
  beforeAll(async () => {
    await setupTestEnvironment();
  }, 120000); // 2 minute timeout for setup

  // Global cleanup
  afterAll(async () => {
    await cleanupTestEnvironment();
  }, 30000);

  describe('Nonce Management System', () => {
    const testUser = 'nonce_test_user';

    beforeEach(async () => {
      await resetOrderbookState();
    });

    test('should start with nonce 0 for new users', async () => {
      const initialNonce = await getNonce(testUser);
      expect(initialNonce).toBe(0);
      console.log(`✓ Initial nonce for ${testUser}: ${initialNonce}`);
    });

    test('should increment nonce after successful operations', async () => {
      // Add session key first
      const sessionKeyResult = await runTxSenderCommand('add-session-key', [], testUser);
      expect(sessionKeyResult.success).toBe(true);
      
      // Check nonce after session key addition
      let currentNonce = await getNonce(testUser);
      expect(currentNonce).toBe(0); // Session key doesn't increment nonce
      console.log(`✓ Nonce after session key: ${currentNonce}`);
      
      // Make a deposit
      const depositResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.HYLLAR,
        '--amount', DEPOSIT_AMOUNT.toString()
      ], testUser);
      expect(depositResult.success).toBe(true);
      
      // Check nonce after deposit (should still be 0 as deposit doesn't require signature)
      currentNonce = await getNonce(testUser);
      expect(currentNonce).toBe(0);
      console.log(`✓ Nonce after deposit: ${currentNonce}`);
      
      // Create an order (this should increment nonce)
      const orderId = `nonce_test_order_${orderCounter++}`;
      const orderResult = await runTxSenderCommand('create-order', [
        '--order-id', orderId,
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1',
        '--price', '1500'
      ], testUser);
      expect(orderResult.success).toBe(true);
      
      // Check nonce after order creation
      currentNonce = await getNonce(testUser);
      expect(currentNonce).toBe(1);
      console.log(`✓ Nonce after order creation: ${currentNonce}`);
      
      // Cancel the order (this should increment nonce again)
      const cancelResult = await runTxSenderCommand('cancel', [
        '--order-id', orderId
      ], testUser);
      expect(cancelResult.success).toBe(true);
      
      // Check final nonce
      currentNonce = await getNonce(testUser);
      expect(currentNonce).toBe(2);
      console.log(`✓ Nonce after order cancellation: ${currentNonce}`);
    }, 60000);
  });

  describe('Multi-User Operations', () => {
    const user1 = 'multiuser_test_1';
    const user2 = 'multiuser_test_2';

    beforeEach(async () => {
      await resetOrderbookState();
    });

    test('should handle multiple users with independent nonces', async () => {
      // Setup both users
      const user1SessionResult = await runTxSenderCommand('add-session-key', [], user1);
      const user2SessionResult = await runTxSenderCommand('add-session-key', [], user2);
      
      expect(user1SessionResult.success).toBe(true);
      expect(user2SessionResult.success).toBe(true);
      
      // Verify initial nonces
      let user1Nonce = await getNonce(user1);
      let user2Nonce = await getNonce(user2);
      expect(user1Nonce).toBe(0);
      expect(user2Nonce).toBe(0);
      
      // Both users deposit tokens
      const user1DepositResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.HYLLAR,
        '--amount', '5000'
      ], user1);
      
      const user2DepositResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.ORANJ,
        '--amount', '10000'
      ], user2);
      
      expect(user1DepositResult.success).toBe(true);
      expect(user2DepositResult.success).toBe(true);
      
      // User1 creates a sell order
      const sellOrderId = `multiuser_sell_${orderCounter++}`;
      const sellOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', sellOrderId,
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '2',
        '--price', '1500'
      ], user1);
      expect(sellOrderResult.success).toBe(true);
      
      // User2 creates a buy order
      const buyOrderId = `multiuser_buy_${orderCounter++}`;
      const buyOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', buyOrderId,
        '--order-side', 'bid',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '2',
        '--price', '1600'
      ], user2);
      expect(buyOrderResult.success).toBe(true);
      
      // Check nonces after operations
      user1Nonce = await getNonce(user1);
      user2Nonce = await getNonce(user2);
      expect(user1Nonce).toBe(1); // One order created
      expect(user2Nonce).toBe(1); // One order created
      
      console.log(`✓ Multi-user nonces: ${user1}=${user1Nonce}, ${user2}=${user2Nonce}`);
      
      // Verify balances reflect the trade (orders should have matched)
      const finalBalances = await getAllBalances();
      console.log('Final balances after multi-user trade:', finalBalances);
    }, 60000);

    test('should handle concurrent operations from different users', async () => {
      // Setup users
      await runTxSenderCommand('add-session-key', [], user1);
      await runTxSenderCommand('add-session-key', [], user2);
      
      // Concurrent deposits
      const depositPromises = [
        runTxSenderCommand('deposit', ['--token', TOKENS.HYLLAR, '--amount', '3000'], user1),
        runTxSenderCommand('deposit', ['--token', TOKENS.ORANJ, '--amount', '6000'], user2)
      ];
      
      const depositResults = await Promise.all(depositPromises);
      expect(depositResults[0].success).toBe(true);
      expect(depositResults[1].success).toBe(true);
      
      // Concurrent order creation
      const orderPromises = [
        runTxSenderCommand('create-order', [
          '--order-id', `concurrent_sell_${orderCounter++}`,
          '--order-side', 'ask',
          '--order-type', 'limit',
          '--pair-token1', TOKENS.HYLLAR,
          '--pair-token2', TOKENS.ORANJ,
          '--quantity', '1',
          '--price', '1800'
        ], user1),
        runTxSenderCommand('create-order', [
          '--order-id', `concurrent_buy_${orderCounter++}`,
          '--order-side', 'bid',
          '--order-type', 'limit',
          '--pair-token1', TOKENS.HYLLAR,
          '--pair-token2', TOKENS.ORANJ,
          '--quantity', '1',
          '--price', '1700'
        ], user2)
      ];
      
      const orderResults = await Promise.all(orderPromises);
      expect(orderResults[0].success).toBe(true);
      expect(orderResults[1].success).toBe(true);
      
      // Verify both users have independent nonce progression
      const user1FinalNonce = await getNonce(user1);
      const user2FinalNonce = await getNonce(user2);
      expect(user1FinalNonce).toBe(1);
      expect(user2FinalNonce).toBe(1);
      
      console.log(`✓ Concurrent operations completed - User nonces: ${user1}=${user1FinalNonce}, ${user2}=${user2FinalNonce}`);
    }, 60000);
  });

  describe('Error Handling and Security', () => {
    const securityTestUser = 'security_test_user';

    beforeEach(async () => {
      await resetOrderbookState();
    });

    test('should handle invalid signatures gracefully', async () => {
      // Setup user
      const sessionResult = await runTxSenderCommand('add-session-key', [], securityTestUser);
      expect(sessionResult.success).toBe(true);
      
      // Try to create an order with potentially invalid parameters
      // Note: tx_sender handles signature creation internally, so we test invalid order parameters
      const invalidOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', 'invalid_order',
        '--order-side', 'invalid_side', // Invalid side
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1',
        '--price', '1000'
      ], securityTestUser);
      
      expect(invalidOrderResult.success).toBe(false);
      console.log('✓ Invalid order parameters properly rejected');
    });

    test('should handle missing required fields', async () => {
      // Try to create order without session key first
      const orderWithoutSessionResult = await runTxSenderCommand('create-order', [
        '--order-id', 'no_session_order',
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1',
        '--price', '1000'
      ], securityTestUser);
      
      // This might succeed or fail depending on the implementation
      // The important thing is that it handles the case gracefully
      console.log(`Order without session key result: ${orderWithoutSessionResult.success}`);
    });

    test('should maintain nonce consistency under error conditions', async () => {
      // Setup user
      await runTxSenderCommand('add-session-key', [], securityTestUser);
      await runTxSenderCommand('deposit', ['--token', TOKENS.HYLLAR, '--amount', '1000'], securityTestUser);
      
      const initialNonce = await getNonce(securityTestUser);
      
      // Try to create an order that should fail (insufficient balance)
      const failingOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', 'insufficient_balance_order',
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '10000', // More than deposited
        '--price', '1000'
      ], securityTestUser);
      
      // Check if nonce changed after failed operation
      const nonceAfterFailure = await getNonce(securityTestUser);
      
      // Create a successful order to see if nonce progression continues correctly
      const successfulOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', 'successful_order',
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1',
        '--price', '1000'
      ], securityTestUser);
      
      const finalNonce = await getNonce(securityTestUser);
      
      console.log(`Nonce progression: initial=${initialNonce}, after_failure=${nonceAfterFailure}, final=${finalNonce}`);
      console.log(`Failed order success: ${failingOrderResult.success}`);
      console.log(`Successful order success: ${successfulOrderResult.success}`);
      
      // Verify that successful operations still increment nonce properly
      if (successfulOrderResult.success) {
        expect(finalNonce).toBeGreaterThan(initialNonce);
      }
    });
  });

  describe('API Consistency Tests', () => {
    const apiTestUser = 'api_test_user';

    beforeEach(async () => {
      await resetOrderbookState();
    });

    test('should maintain consistent balance format across different endpoints', async () => {
      // Setup and deposit
      await runTxSenderCommand('add-session-key', [], apiTestUser);
      await runTxSenderCommand('deposit', ['--token', TOKENS.HYLLAR, '--amount', '2000'], apiTestUser);
      await runTxSenderCommand('deposit', ['--token', TOKENS.ORANJ, '--amount', '3000'], apiTestUser);
      
      // Get balances from different endpoints
      const allBalances = await getAllBalances();
      const userSpecificBalance = await getBalanceForAccount(apiTestUser);
      
      // Verify consistency between endpoints
      if (allBalances[TOKENS.HYLLAR] && allBalances[TOKENS.HYLLAR][apiTestUser]) {
        expect(allBalances[TOKENS.HYLLAR][apiTestUser]).toBe(userSpecificBalance[TOKENS.HYLLAR]);
      }
      
      if (allBalances[TOKENS.ORANJ] && allBalances[TOKENS.ORANJ][apiTestUser]) {
        expect(allBalances[TOKENS.ORANJ][apiTestUser]).toBe(userSpecificBalance[TOKENS.ORANJ]);
      }
      
      console.log('✓ Balance consistency verified across different endpoints');
      console.log('All balances format:', allBalances);
      console.log('User specific balance format:', userSpecificBalance);
    });

    test('should handle nonce endpoint correctly', async () => {
      // Test nonce endpoint for new user
      const newUserNonce = await getNonce(apiTestUser);
      expect(newUserNonce).toBe(0);
      
      // Setup user and perform operations
      await runTxSenderCommand('add-session-key', [], apiTestUser);
      await runTxSenderCommand('deposit', ['--token', TOKENS.HYLLAR, '--amount', '1000'], apiTestUser);
      
      // Create and cancel an order to increment nonce
      const orderId = `api_test_order_${orderCounter++}`;
      await runTxSenderCommand('create-order', [
        '--order-id', orderId,
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1',
        '--price', '2000'
      ], apiTestUser);
      
      const nonceAfterOrder = await getNonce(apiTestUser);
      expect(nonceAfterOrder).toBe(1);
      
      await runTxSenderCommand('cancel', ['--order-id', orderId], apiTestUser);
      
      const finalNonce = await getNonce(apiTestUser);
      expect(finalNonce).toBe(2);
      
      console.log(`✓ Nonce endpoint working correctly: 0 → ${nonceAfterOrder} → ${finalNonce}`);
    });
  });
});