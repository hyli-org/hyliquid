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

      // Step 2: Deposit ORANJ tokens
      console.log('Step 2: Depositing ORANJ tokens...');
      const depositOranjResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.ORANJ,
        '--amount', DEPOSIT_AMOUNT.toString()
      ]);
      expect(depositOranjResult.success).toBe(true);
      console.log(`✓ Deposited ${DEPOSIT_AMOUNT} ${TOKENS.ORANJ} tokens`);

      // Step 3: Create sell order
      console.log('Step 3: Creating sell order...');
      const sellOrderId = `sell_${orderCounter++}`;
      const sellOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', sellOrderId,
        '--order-type', 'Sell',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', ORDER_QUANTITY.toString(),
        '--price', SELL_PRICE.toString()
      ]);
      expect(sellOrderResult.success).toBe(true);
      console.log(`✓ Created sell order ${sellOrderId}: ${ORDER_QUANTITY} ${TOKENS.HYLLAR} at ${SELL_PRICE} ${TOKENS.ORANJ}`);

      // Step 4: Create buy order
      console.log('Step 4: Creating buy order...');
      const buyOrderId = `buy_${orderCounter++}`;
      const buyOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', buyOrderId,
        '--order-type', 'Buy',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', ORDER_QUANTITY.toString(),
        '--price', BUY_PRICE.toString()
      ]);
      expect(buyOrderResult.success).toBe(true);
      console.log(`✓ Created buy order ${buyOrderId}: ${ORDER_QUANTITY} ${TOKENS.HYLLAR} at ${BUY_PRICE} ${TOKENS.ORANJ}`);

      console.log('🎉 Complete trading workflow executed successfully!');
    }, 60000); // 1 minute timeout per test

    test('should handle additional deposits', async () => {
      console.log('Testing additional deposits...');
      
      const smallAmount = 100;
      
      // Make small deposits to test system robustness
      const depositResult = await runTxSenderCommand('deposit', [
        '--token', TOKENS.HYLLAR,
        '--amount', smallAmount.toString()
      ]);
      
      expect(depositResult.success).toBe(true);
      console.log(`✓ Additional deposit of ${smallAmount} ${TOKENS.HYLLAR} successful`);
    }, 30000);

    test('should create additional orders with different parameters', async () => {
      console.log('Testing additional order creation...');
      
      // Create market order (without price)
      const marketOrderId = `market_${orderCounter++}`;
      const marketOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', marketOrderId,
        '--order-type', 'Buy',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '1'
        // No price specified - should be market order
      ]);
      
      // Create limit order with different price
      const limitOrderId = `limit_${orderCounter++}`;
      const limitOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', limitOrderId,
        '--order-type', 'Sell',
        '--pair-token1', TOKENS.HYLLAR,
        '--pair-token2', TOKENS.ORANJ,
        '--quantity', '2',
        '--price', '1500'
      ]);
      
      expect(marketOrderResult.success).toBe(true);
      expect(limitOrderResult.success).toBe(true);
      console.log(`✓ Created market order ${marketOrderId} and limit order ${limitOrderId}`);
    }, 30000);
  });

  describe('Error Handling', () => {
    test('should handle invalid command arguments gracefully', async () => {
      console.log('Testing error handling...');
      
      // Try to create order with invalid order type
      const invalidOrderId = `invalid_${orderCounter++}`;
      const invalidOrderResult = await runTxSenderCommand('create-order', [
        '--order-id', invalidOrderId,
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
        '--order-type', 'Buy'
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
          '--order-type', i % 2 === 0 ? 'Buy' : 'Sell',
          '--pair-token1', TOKENS.HYLLAR,
          '--pair-token2', TOKENS.ORANJ,
          '--quantity', '1',
          '--price', (1000 + i * 10).toString()
        ]);
        
        expect(result.success).toBe(true);
        console.log(`✓ Sequential order ${orderId} created successfully`);
      }
      
      console.log('✓ All sequential commands completed successfully');
    }, 45000);
  });
});
