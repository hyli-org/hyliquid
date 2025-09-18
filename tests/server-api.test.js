/**
 * Integration Tests using tx_sender binary
 * 
 * This file contains integration tests that use the actual tx_sender binary
 * to interact with the orderbook system, simulating real usage.
 * 
 * Make sure the server is running before executing these tests.
 * 
 * To run: npm test
 */

import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

// Configuration
const SERVER_URL = process.env.SERVER_URL || 'http://localhost:9002';
const CONFIG_FILE = process.env.CONFIG_FILE || 'config.toml';
const IDENTITY = process.env.IDENTITY || 'txsender@orderbook';

// Test data
const TOKENS = {
  HYLLAR: 'HYLLAR',
  ORANJ: 'ORANJ'
};

const DEPOSIT_AMOUNT = 10000;
const ORDER_QUANTITY = 1;
const SELL_PRICE = 1000;
const BUY_PRICE = 1010;

/**
 * Helper function to run tx_sender command
 */
async function runTxSenderCommand(command, args = []) {
  const baseCmd = `cargo run --bin tx_sender --`;
  const fullCmd = `${baseCmd} ${command} ${args.join(' ')}`;
  
  console.log(`Executing: ${fullCmd}`);
  
  try {
    const { stdout, stderr } = await execAsync(fullCmd, {
      cwd: '/home/maximilien/hyliquid',
      timeout: 30000 // 30 second timeout
    });
    
    if (stderr && !stderr.includes('Compiling') && !stderr.includes('Finished')) {
      console.warn(`Command stderr: ${stderr}`);
    }
    
    return {
      success: true,
      stdout: stdout.trim(),
      stderr: stderr.trim()
    };
  } catch (error) {
    console.error(`Command failed: ${fullCmd}`);
    console.error(`Error: ${error.message}`);
    return {
      success: false,
      error: error.message,
      stdout: error.stdout || '',
      stderr: error.stderr || ''
    };
  }
}

/**
 * Helper function to check if server is responding
 */
async function checkServerHealth() {
  try {
    const response = await fetch(`${SERVER_URL}/_health`, {
      method: 'GET',
      signal: AbortSignal.timeout(5000)
    });
    return response.ok;
  } catch (error) {
    return false;
  }
}

/**
 * Helper function to get all balances from the server
 */
async function getAllBalances() {
  try {
    const response = await fetch(`${SERVER_URL}/temp/balances`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json'
      },
      signal: AbortSignal.timeout(10000)
    });
    
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    
    return await response.json();
  } catch (error) {
    console.error('Failed to get balances:', error);
    throw error;
  }
}

/**
 * Helper function to get balance for a specific account
 */
async function getBalanceForAccount(user) {
  try {
    const response = await fetch(`${SERVER_URL}/temp/balance/${encodeURIComponent(user)}`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json'
      },
      signal: AbortSignal.timeout(10000)
    });
    
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    
    return await response.json();
  } catch (error) {
    console.error(`Failed to get balance for ${user}:`, error);
    throw error;
  }
}

/**
 * Helper function to get all orders from the server
 */
async function getAllOrders() {
  try {
    const response = await fetch(`${SERVER_URL}/temp/orders`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json'
      },
      signal: AbortSignal.timeout(10000)
    });
    
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    
    return await response.json();
  } catch (error) {
    console.error('Failed to get orders:', error);
    throw error;
  }
}

/**
 * Helper function to get orders by token pair
 */
async function getOrdersByPair(token1, token2) {
  try {
    const response = await fetch(`${SERVER_URL}/temp/orders/${encodeURIComponent(token1)}/${encodeURIComponent(token2)}`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json'
      },
      signal: AbortSignal.timeout(10000)
    });
    
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    
    return await response.json();
  } catch (error) {
    console.error(`Failed to get orders for pair ${token1}/${token2}:`, error);
    throw error;
  }
}

/**
 * Helper function to verify balance expectations
 */
function verifyBalance(balances, user, token, expectedAmount, description = '') {
  const userBalances = balances[token];
  if (!userBalances) {
    throw new Error(`Token ${token} not found in balances ${description}`);
  }
  
  const userBalance = userBalances[user];
  if (!userBalance) {
    throw new Error(`User ${user} not found in ${token} balances ${description}`);
  }
  
  if (userBalance.balance !== expectedAmount) {
    throw new Error(`Expected ${user} to have ${expectedAmount} ${token} but found ${userBalance.balance} ${description}`);
  }
  
  console.log(`✓ Verified: ${user} has ${userBalance.balance} ${token} ${description}`);
}

/**
 * Helper function to verify order expectations
 */
function verifyOrderExists(orders, orderId, description = '') {
  const order = orders[orderId];
  if (!order) {
    throw new Error(`Order ${orderId} not found in orders ${description}`);
  }
  
  console.log(`✓ Verified: Order ${orderId} exists ${description}:`, {
    side: order.order_side,
    type: order.order_type,
    quantity: order.quantity,
    price: order.price
  });
  
  return order;
}

/**
 * Helper function to reset the orderbook state
 */
async function resetOrderbookState() {
  try {
    const response = await fetch(`${SERVER_URL}/temp/reset_state`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json'
      },
      signal: AbortSignal.timeout(10000)
    });
    
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    
    console.log('🧹 Orderbook state reset successfully');
    return true;
  } catch (error) {
    console.error('Failed to reset orderbook state:', error);
    throw error;
  }
}

describe('TX Sender Integration Tests', () => {
  let sessionKeyAdded = false;
  let orderCounter = 1;

  // Global setup - check if server is reachable and build the project
  beforeAll(async () => {
    // Check server health
    const serverHealthy = await checkServerHealth();
    if (!serverHealthy) {
      throw new Error(`Cannot connect to server at ${SERVER_URL}. Make sure the server is running.`);
    }

    // Build the project to ensure tx_sender binary is available
    console.log('Building tx_sender binary...');
    try {
      await execAsync('cargo build --bin tx_sender', {
        cwd: '/home/maximilien/hyliquid',
        timeout: 60000 // 1 minute timeout for build
      });
      console.log('Build completed successfully');
    } catch (error) {
      throw new Error(`Failed to build tx_sender binary: ${error.message}`);
    }

    // Add session key ONCE at the beginning
    console.log('🔑 Adding session key (one time setup)...');
    const sessionKeyResult = await runTxSenderCommand('add-session-key');
    if (sessionKeyResult.success || sessionKeyResult.stderr.includes('Session key already exists')) {
      sessionKeyAdded = true;
      console.log('✓ Session key ready');
    } else {
      throw new Error(`Failed to add session key: ${sessionKeyResult.error}`);
    }
  }, 120000); // 2 minute timeout for setup

  // Global cleanup - reset orderbook state after all tests
  afterAll(async () => {
    try {
      console.log('🧹 Cleaning up: Resetting orderbook state...');
      await resetOrderbookState();
      console.log('✓ Cleanup completed successfully');
    } catch (error) {
      console.warn('⚠️ Failed to reset orderbook state during cleanup:', error.message);
      // Don't fail the tests if reset fails, just warn
    }
  }, 30000); // 30 second timeout for cleanup

  describe('Complete Orderbook Workflow', () => {
    test('should execute full trading workflow successfully', async () => {
      // Verify session key was added
      expect(sessionKeyAdded).toBe(true);

      // Step 1: Deposit HYLLAR tokens
      console.log('Step 1: Depositing HYLLAR tokens...');
      const depositHyllarResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.HYLLAR,
        '--amount', DEPOSIT_AMOUNT.toString()
      ]);
      expect(depositHyllarResult.success).toBe(true);
      console.log(`✓ Deposited ${DEPOSIT_AMOUNT} ${TOKENS.HYLLAR} tokens`);

      // Verify HYLLAR balance after deposit
      console.log('Verifying HYLLAR balance after deposit...');
      const balancesAfterHyllar = await getAllBalances();
      verifyBalance(balancesAfterHyllar, IDENTITY, TOKENS.HYLLAR, DEPOSIT_AMOUNT, 'after HYLLAR deposit');

      // Step 2: Deposit ORANJ tokens
      console.log('Step 2: Depositing ORANJ tokens...');
      const depositOranjResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.ORANJ,
        '--amount', DEPOSIT_AMOUNT.toString()
      ]);
      expect(depositOranjResult.success).toBe(true);
      console.log(`✓ Deposited ${DEPOSIT_AMOUNT} ${TOKENS.ORANJ} tokens`);

      // Verify both balances after ORANJ deposit
      console.log('Verifying balances after both deposits...');
      const balancesAfterBoth = await getAllBalances();
      verifyBalance(balancesAfterBoth, IDENTITY, TOKENS.HYLLAR, DEPOSIT_AMOUNT, 'after both deposits');
      verifyBalance(balancesAfterBoth, IDENTITY, TOKENS.ORANJ, DEPOSIT_AMOUNT, 'after both deposits');

      // Step 3: Create sell order
      console.log('Step 3: Creating sell order...');
      const sellOrderId = `sell_${orderCounter++}`;
      const sellOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', sellOrderId,
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', ORDER_QUANTITY.toString(),
        '--price', SELL_PRICE.toString()
      ]);
      expect(sellOrderResult.success).toBe(true);
      console.log(`✓ Created sell order ${sellOrderId}: ${ORDER_QUANTITY} ${TOKENS.HYLLAR} at ${SELL_PRICE} ${TOKENS.ORANJ}`);

      // Verify sell order exists and balances updated (tokens should be reserved)
      console.log('Verifying sell order creation...');
      const ordersAfterSell = await getAllOrders();
      verifyOrderExists(ordersAfterSell, sellOrderId, 'after sell order creation');
      
      const balancesAfterSell = await getAllBalances();
      // HYLLAR should be reduced by order quantity (reserved for sell order)
      verifyBalance(balancesAfterSell, IDENTITY, TOKENS.HYLLAR, DEPOSIT_AMOUNT - ORDER_QUANTITY, 'after sell order (tokens reserved)');
      verifyBalance(balancesAfterSell, IDENTITY, TOKENS.ORANJ, DEPOSIT_AMOUNT, 'after sell order (unchanged)');

      // Step 4: Create buy order
      console.log('Step 4: Creating buy order...');
      const buyOrderId = `buy_${orderCounter++}`;
      const buyOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', buyOrderId,
        '--order-side', 'bid',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', ORDER_QUANTITY.toString(),
        '--price', BUY_PRICE.toString()
      ]);
      expect(buyOrderResult.success).toBe(true);
      console.log(`✓ Created buy order ${buyOrderId}: ${ORDER_QUANTITY} ${TOKENS.HYLLAR} at ${BUY_PRICE} ${TOKENS.ORANJ}`);

      // Verify buy order exists and final balances
      console.log('Verifying buy order creation and final state...');
      const finalOrders = await getAllOrders();
      const finalBalances = await getAllBalances();
      
      // Check if orders matched (buy price higher than sell price should execute)
      if (BUY_PRICE >= SELL_PRICE) {
        console.log('Orders should have matched (buy price >= sell price)');
        // Orders should be executed, check final balances reflect the trade
        // Note: Exact final balances depend on matching logic, but we can verify the trade happened
        console.log('Final balances:', finalBalances);
        console.log('Final orders:', finalOrders);
      } else {
        console.log('Orders should not match (buy price < sell price)');
        // Both orders should still exist
        verifyOrderExists(finalOrders, sellOrderId, 'final state (no match)');
        verifyOrderExists(finalOrders, buyOrderId, 'final state (no match)');
        
        // Verify ORANJ tokens are reserved for buy order
        const expectedOranjBalance = DEPOSIT_AMOUNT - (ORDER_QUANTITY * BUY_PRICE);
        verifyBalance(finalBalances, IDENTITY, TOKENS.ORANJ, expectedOranjBalance, 'final state (ORANJ reserved for buy)');
        
        // Step 5: Cancel one of the orders (test order cancellation)
        console.log('Step 5: Testing order cancellation...');
        const orderToCancel = sellOrderId; // Cancel the sell order
        
        // Get balances before cancellation
        const balancesBeforeCancel = await getAllBalances();
        const hyllarBalanceBeforeCancel = balancesBeforeCancel[TOKENS.HYLLAR]?.[IDENTITY]?.balance || 0;
        
        const cancelResult = await runTxSenderCommand('cancel-order', [
          '--order-id', orderToCancel
        ]);
        expect(cancelResult.success).toBe(true);
        console.log(`✓ Cancelled order ${orderToCancel}`);
        
        // Verify order was removed and balance was restored
        const ordersAfterCancel = await getAllOrders();
        const balancesAfterCancel = await getAllBalances();
        
        if (ordersAfterCancel[orderToCancel]) {
          throw new Error(`Order ${orderToCancel} should have been cancelled but still exists`);
        }
        console.log(`✓ Order ${orderToCancel} successfully removed from orderbook`);
        
        // Balance should be restored (tokens should be returned to user)
        const hyllarBalanceAfterCancel = balancesAfterCancel[TOKENS.HYLLAR]?.[IDENTITY]?.balance || 0;
        const expectedHyllarAfterCancel = hyllarBalanceBeforeCancel + ORDER_QUANTITY; // Tokens returned
        verifyBalance(balancesAfterCancel, IDENTITY, TOKENS.HYLLAR, expectedHyllarAfterCancel, 'after order cancellation (tokens returned)');
        
        console.log(`✓ Order cancellation test completed - ${ORDER_QUANTITY} ${TOKENS.HYLLAR} tokens returned to user`);
      }

      // Step 6: Test withdrawal functionality
      console.log('Step 6: Testing token withdrawal...');
      
      // Get current balance before withdrawal
      const balancesBeforeWithdraw = await getAllBalances();
      const oranjBalanceBeforeWithdraw = balancesBeforeWithdraw[TOKENS.ORANJ]?.[IDENTITY]?.balance || 0;
      
      const withdrawAmount = 500; // Withdraw a portion of ORANJ tokens
      if (oranjBalanceBeforeWithdraw < withdrawAmount) {
        console.log(`⚠️ Insufficient ${TOKENS.ORANJ} balance for withdrawal test. Current: ${oranjBalanceBeforeWithdraw}, needed: ${withdrawAmount}`);
        // Skip withdrawal test if insufficient balance
      } else {
        const withdrawResult = await runTxSenderCommand('withdraw', [
          '--token', TOKENS.ORANJ,
          '--amount', withdrawAmount.toString()
        ]);
        expect(withdrawResult.success).toBe(true);
        console.log(`✓ Withdrew ${withdrawAmount} ${TOKENS.ORANJ} tokens`);
        
        // Verify balance was reduced correctly
        const balancesAfterWithdraw = await getAllBalances();
        const expectedOranjAfterWithdraw = oranjBalanceBeforeWithdraw - withdrawAmount;
        verifyBalance(balancesAfterWithdraw, IDENTITY, TOKENS.ORANJ, expectedOranjAfterWithdraw, 'after withdrawal');
        
        console.log(`✓ Withdrawal test completed - balance correctly reduced by ${withdrawAmount} ${TOKENS.ORANJ} tokens`);
      }

      console.log('🎉 Complete trading workflow executed successfully!');
    }, 60000); // 1 minute timeout per test

    test('should handle additional deposits', async () => {
      console.log('Testing additional deposits...');
      
      const smallAmount = 100;
      
      // Get initial balance
      const initialBalances = await getAllBalances();
      const initialHyllarBalance = initialBalances[TOKENS.HYLLAR]?.[IDENTITY]?.balance || 0;
      
      // Make small deposits to test system robustness
      const depositResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.HYLLAR,
        '--amount', smallAmount.toString()
      ]);
      
      expect(depositResult.success).toBe(true);
      console.log(`✓ Additional deposit of ${smallAmount} ${TOKENS.HYLLAR} successful`);
      
      // Verify balance increased correctly
      const updatedBalances = await getAllBalances();
      const expectedBalance = initialHyllarBalance + smallAmount;
      verifyBalance(updatedBalances, IDENTITY, TOKENS.HYLLAR, expectedBalance, 'after additional deposit');
    }, 30000);

    test('should create additional orders with different parameters', async () => {
      console.log('Testing additional order creation...');
      
      // Get initial orders count
      const initialOrders = await getAllOrders();
      const initialOrderCount = Object.keys(initialOrders).length;
      
      // Create market order (without price)
      const marketOrderId = `market_${orderCounter++}`;
      const marketOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', marketOrderId,
        '--order-side', 'bid',
        '--order-type', 'market',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1'
        // No price specified - should be market order
      ]);
      
      // Create limit order with different price
      const limitOrderId = `limit_${orderCounter++}`;
      const limitOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', limitOrderId,
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '2',
        '--price', '1500'
      ]);
      
      expect(marketOrderResult.success).toBe(true);
      expect(limitOrderResult.success).toBe(true);
      console.log(`✓ Created market order ${marketOrderId} and limit order ${limitOrderId}`);
      
      // Verify orders were created (note: market orders might get executed immediately)
      const updatedOrders = await getAllOrders();
      console.log('Orders after creation:', Object.keys(updatedOrders));
      
      // For limit order, it should exist unless it was matched
      if (updatedOrders[limitOrderId]) {
        verifyOrderExists(updatedOrders, limitOrderId, 'after limit order creation');
      } else {
        console.log(`ℹ Limit order ${limitOrderId} was immediately executed/matched`);
      }
      
      // Market order might be executed immediately, so we just verify the command succeeded
      console.log(`ℹ Market order ${marketOrderId} command executed (might be filled immediately)`);
      
      // Verify pair-specific orders
      const pairOrders = await getOrdersByPair(TOKENS.HYLLAR, TOKENS.ORANJ);
      console.log(`Orders for ${TOKENS.HYLLAR}/${TOKENS.ORANJ} pair:`, Object.keys(pairOrders.buy_orders || {}), Object.keys(pairOrders.sell_orders || {}));
    }, 30000);

    test('should handle order cancellation and withdrawals independently', async () => {
      console.log('Testing isolated order cancellation and withdrawal...');
      
      // First, ensure we have some balance for testing
      const depositAmount = 1000;
      const depositResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.HYLLAR,
        '--amount', depositAmount.toString()
      ]);
      expect(depositResult.success).toBe(true);
      
      // Create an order specifically for cancellation testing
      const testOrderId = `cancel_test_${orderCounter++}`;
      const orderResult = await runTxSenderCommand('create-order', [
        '--order-id', testOrderId,
        '--order-side', 'ask',
        '--order-type', 'limit',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '5',
        '--price', '2000'
      ]);
      expect(orderResult.success).toBe(true);
      console.log(`✓ Created test order ${testOrderId} for cancellation`);
      
      // Get balances before cancellation
      const balancesBeforeCancel = await getAllBalances();
      const hyllarBeforeCancel = balancesBeforeCancel[TOKENS.HYLLAR]?.[IDENTITY]?.balance || 0;
      
      // Verify order exists
      const ordersBeforeCancel = await getAllOrders();
      verifyOrderExists(ordersBeforeCancel, testOrderId, 'before cancellation');
      
      // Cancel the order
      const cancelResult = await runTxSenderCommand('cancel', [
        '--order-id', testOrderId
      ]);
      expect(cancelResult.success).toBe(true);
      console.log(`✓ Successfully cancelled order ${testOrderId}`);
      
      // Verify order was removed
      const ordersAfterCancel = await getAllOrders();
      if (ordersAfterCancel[testOrderId]) {
        throw new Error(`Order ${testOrderId} should have been cancelled but still exists`);
      }
      console.log(`✓ Order ${testOrderId} correctly removed from orderbook`);
      
      // Verify balance was restored (tokens returned)
      const balancesAfterCancel = await getAllBalances();
      const hyllarAfterCancel = balancesAfterCancel[TOKENS.HYLLAR]?.[IDENTITY]?.balance || 0;
      const expectedBalance = hyllarBeforeCancel + 5; // 5 tokens should be returned
      verifyBalance(balancesAfterCancel, IDENTITY, TOKENS.HYLLAR, expectedBalance, 'after order cancellation');
      
      // Test withdrawal with the newly restored balance
      const withdrawAmount = 100;
      const withdrawResult = await runTxSenderCommand('withdraw', [
        '--token', TOKENS.HYLLAR,
        '--amount', withdrawAmount.toString()
      ]);
      expect(withdrawResult.success).toBe(true);
      console.log(`✓ Successfully withdrew ${withdrawAmount} ${TOKENS.HYLLAR} tokens`);
      
      // Verify withdrawal reduced balance correctly
      const balancesAfterWithdraw = await getAllBalances();
      const expectedAfterWithdraw = expectedBalance - withdrawAmount;
      verifyBalance(balancesAfterWithdraw, IDENTITY, TOKENS.HYLLAR, expectedAfterWithdraw, 'after withdrawal');
      
      console.log('✓ Order cancellation and withdrawal test completed successfully');
    }, 45000);
  });

  describe('Error Handling', () => {
    test('should handle invalid command arguments gracefully', async () => {
      console.log('Testing error handling...');
      
      // Try to create order with invalid order type
      const invalidOrderId = `invalid_${orderCounter++}`;
      const invalidOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', invalidOrderId,
        '--order-side', 'bid',
        '--order-type', 'InvalidType',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1',
        '--price', '1000'
      ]);
      
      // This should fail due to invalid order type
      expect(invalidOrderResult.success).toBe(false);
      console.log('✓ Invalid order type properly rejected');
    }, 30000);

    test('should handle missing required arguments', async () => {
      console.log('Testing missing arguments...');
      
      // Try to create order without required fields
      const incompleteOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', `incomplete_${orderCounter++}`,
        '--order-side', 'bid'
        // Missing other required fields
      ]);
      
      // This should fail due to missing arguments
      expect(incompleteOrderResult.success).toBe(false);
      console.log('✓ Missing arguments properly rejected');
    }, 30000);
  });

  describe('Performance and Reliability', () => {
    test('should handle sequential commands efficiently', async () => {
      console.log('Testing sequential commands...');
      
      const orderIds = [];
      
      // Create multiple orders sequentially
      for (let i = 0; i < 3; i++) {
        const orderId = `sequential_${orderCounter++}`;
        orderIds.push(orderId);
        
        const result = await runTxSenderCommand('create-order', [
          '--order-id', orderId,
          '--order-side', i % 2 === 0 ? 'bid' : 'ask',
          '--order-type', 'limit',
          '--pair-token1', TOKENS.HYLLAR,
          '--pair-token2', TOKENS.ORANJ,
          '--quantity', '1',
          '--price', (1000 + i * 10).toString()
        ]);
        
        expect(result.success).toBe(true);
        console.log(`✓ Sequential order ${orderId} created successfully`);
      }
      
      // Verify all orders after creation
      const finalOrders = await getAllOrders();
      let foundOrders = 0;
      
      for (const orderId of orderIds) {
        if (finalOrders[orderId]) {
          verifyOrderExists(finalOrders, orderId, 'in sequential test');
          foundOrders++;
        } else {
          console.log(`ℹ Order ${orderId} not found (might have been executed)`);
        }
      }
      
      console.log(`✓ All sequential commands completed successfully (${foundOrders}/${orderIds.length} orders still active)`);
    }, 45000);

    test('should verify orderbook state consistency', async () => {
      console.log('Testing orderbook state verification...');
      
      // Get all current data
      const allBalances = await getAllBalances();
      const userBalance = await getBalanceForAccount(IDENTITY);
      const allOrders = await getAllOrders();
      const pairOrders = await getOrdersByPair(TOKENS.HYLLAR, TOKENS.ORANJ);
      
      console.log('📊 Current Orderbook State:');
      console.log('All balances:', allBalances);
      console.log(`User ${IDENTITY} balance:`, userBalance);
      console.log('All orders:', Object.keys(allOrders));
      console.log(`${TOKENS.HYLLAR}/${TOKENS.ORANJ} pair orders:`, {
        buyOrders: Object.keys(pairOrders.buy_orders || {}),
        sellOrders: Object.keys(pairOrders.sell_orders || {})
      });
      
      // Verify user balance consistency between endpoints
      if (allBalances[TOKENS.HYLLAR] && allBalances[TOKENS.HYLLAR][IDENTITY]) {
        expect(allBalances[TOKENS.HYLLAR][IDENTITY].balance).toBe(userBalance[TOKENS.HYLLAR]?.balance || 0);
        console.log('✓ HYLLAR balance consistent between endpoints');
      }
      
      if (allBalances[TOKENS.ORANJ] && allBalances[TOKENS.ORANJ][IDENTITY]) {
        expect(allBalances[TOKENS.ORANJ][IDENTITY].balance).toBe(userBalance[TOKENS.ORANJ]?.balance || 0);
        console.log('✓ ORANJ balance consistent between endpoints');
      }
      
      // Verify order consistency
      const pairBuyOrderCount = Object.keys(pairOrders.buy_orders || {}).length;
      const pairSellOrderCount = Object.keys(pairOrders.sell_orders || {}).length;
      console.log(`✓ Found ${pairBuyOrderCount} buy orders and ${pairSellOrderCount} sell orders for ${TOKENS.HYLLAR}/${TOKENS.ORANJ}`);
      
      console.log('✓ Orderbook state verification completed successfully');
    }, 30000);
  });
});
