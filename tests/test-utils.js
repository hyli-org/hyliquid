/**
 * Shared utility functions for HyLiquid Orderbook integration tests
 * 
 * This file contains common helper functions used across all test suites
 * to avoid code duplication and ensure consistency.
 */

import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

// Configuration constants
export const SERVER_URL = process.env.SERVER_URL || 'http://localhost:9002';
export const DEFAULT_IDENTITY = process.env.IDENTITY || 'txsender';

// Test data constants
export const TOKENS = {
  HYLLAR: 'HYLLAR',
  ORANJ: 'ORANJ'
};

/**
 * Helper function to run tx_sender command with optional custom identity
 * @param {string} command - The command to run (e.g., 'create-order', 'deposit')
 * @param {string[]} args - Array of command arguments
 * @param {string} identity - Optional custom identity (defaults to DEFAULT_IDENTITY)
 * @returns {Promise<{success: boolean, stdout: string, stderr: string, error?: string}>}
 */
export async function runTxSenderCommand(command, args = [], identity = DEFAULT_IDENTITY) {
  const baseCmd = `cargo run --bin tx_sender --`;
  const identityArg = `--identity ${identity}`;
  const fullCmd = `${baseCmd} ${identityArg} ${command} ${args.join(' ')}`;
  
  console.log(`Executing: ${fullCmd}`);
  
  try {
    const { stdout, stderr } = await execAsync(fullCmd, {
      cwd: '..',
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
 * @returns {Promise<boolean>} - True if server is healthy, false otherwise
 */
export async function checkServerHealth() {
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
 * Helper function to get nonce for a specific user
 * @param {string} identity - The user identity
 * @returns {Promise<number>} - The current nonce for the user
 */
export async function getNonce(identity) {
  try {
    const response = await fetch(`${SERVER_URL}/nonce`, {
      method: 'GET',
      headers: {
        'x-identity': identity,
        'Content-Type': 'application/json'
      },
      signal: AbortSignal.timeout(10000)
    });
    
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    
    const nonce = await response.text();
    return parseInt(nonce, 10);
  } catch (error) {
    console.error(`Failed to get nonce for ${identity}:`, error);
    throw error;
  }
}

/**
 * Helper function to get all balances from the server
 * @returns {Promise<Object>} - Object containing all user balances by token
 */
export async function getAllBalances() {
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
 * @param {string} user - The user account name
 * @returns {Promise<Object>} - Object containing user's balances by token
 */
export async function getBalanceForAccount(user) {
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
 * @returns {Promise<Object>} - Object containing all orders
 */
export async function getAllOrders() {
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
 * @param {string} token1 - First token in the pair
 * @param {string} token2 - Second token in the pair
 * @returns {Promise<Object>} - Object containing buy and sell orders for the pair
 */
export async function getOrdersByPair(token1, token2) {
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
 * Helper function to reset the orderbook state
 * @param {boolean} recreatePairs - Whether to recreate trading pairs after reset (default: true)
 * @returns {Promise<boolean>} - True if reset was successful
 */
export async function resetOrderbookState(recreatePairs = true) {
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
    
    // Recreate trading pairs after reset
    if (recreatePairs) {
      await setupTradingPairs();
    }
    
    return true;
  } catch (error) {
    console.error('Failed to reset orderbook state:', error);
    throw error;
  }
}

/**
 * Helper function to verify balance expectations
 * @param {Object} balances - The balances object from API
 * @param {string} user - The user account name
 * @param {string} token - The token name
 * @param {number} expectedAmount - Expected balance amount
 * @param {string} description - Optional description for logging
 */
export function verifyBalance(balances, user, token, expectedAmount, description = '') {
  const userBalances = balances[token];
  if (!userBalances) {
    throw new Error(`Token ${token} not found in balances ${description}`);
  }
  
  const userBalance = userBalances[user];
  if (userBalance === undefined) {
    throw new Error(`User ${user} not found in ${token} balances ${description}`);
  }
  
  if (userBalance !== expectedAmount) {
    throw new Error(`Expected ${user} to have ${expectedAmount} ${token} but found ${userBalance} ${description}`);
  }
  
  console.log(`✓ Verified: ${user} has ${userBalance} ${token} ${description}`);
}

/**
 * Helper function to verify order expectations
 * @param {Object} orders - The orders object from API
 * @param {string} orderId - The order ID to verify
 * @param {string} description - Optional description for logging
 * @returns {Object} - The order object if found
 */
export function verifyOrderExists(orders, orderId, description = '') {
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
 * Helper function to build the tx_sender binary once
 * @returns {Promise<void>}
 */
export async function buildTxSender() {
  console.log('Building tx_sender binary...');
  try {
    await execAsync('cargo build --bin tx_sender', {
      cwd: '..',
      timeout: 60000 // 1 minute timeout for build
    });
    console.log('Build completed successfully');
  } catch (error) {
    throw new Error(`Failed to build tx_sender binary: ${error.message}`);
  }
}

/**
 * Helper function to add session key for a user
 * @param {string} identity - The user identity
 * @returns {Promise<boolean>} - True if session key was added successfully
 */
export async function addSessionKey(identity = DEFAULT_IDENTITY) {
  const sessionKeyResult = await runTxSenderCommand('add-session-key', [], identity);
  
  if (sessionKeyResult.success || sessionKeyResult.stderr?.includes('Session key already exists')) {
    console.log(`✓ Session key ready for ${identity}`);
    return true;
  } else {
    throw new Error(`Failed to add session key for ${identity}: ${sessionKeyResult.error}`);
  }
}

/**
 * Helper function to create a trading pair
 * @param {string} token1 - First token in the pair
 * @param {string} token2 - Second token in the pair
 * @param {string} identity - The user identity to use for creating the pair
 * @returns {Promise<boolean>} - True if pair was created successfully
 */
export async function createTradingPair(token1, token2, identity = DEFAULT_IDENTITY) {
  console.log(`Creating trading pair ${token1}/${token2}...`);
  
  const pairResult = await runTxSenderCommand('create-pair', [
    '--pair-token1', token1,
    '--pair-token2', token2
  ], identity);
  
  if (pairResult.success || pairResult.stderr?.includes('already exists')) {
    console.log(`✓ Trading pair ${token1}/${token2} ready`);
    return true;
  } else {
    throw new Error(`Failed to create trading pair ${token1}/${token2}: ${pairResult.error || pairResult.stderr}`);
  }
}

/**
 * Helper function to setup all required trading pairs
 * @param {string} identity - The user identity to use for creating pairs
 * @returns {Promise<void>}
 */
export async function setupTradingPairs(identity = DEFAULT_IDENTITY) {
  console.log('🔧 Setting up trading pairs...');
  
  // Add session key first for the user who will create pairs
  await addSessionKey(identity);
  
  // Create all required trading pairs
  const pairs = [
    [TOKENS.HYLLAR, TOKENS.ORANJ],
    // Add more pairs here as needed
  ];
  
  for (const [token1, token2] of pairs) {
    await createTradingPair(token1, token2, identity);
  }
  
  console.log('✓ All trading pairs set up successfully');
}

/**
 * Helper function to setup test environment
 * Checks server health, builds binary, and sets up trading pairs
 * @returns {Promise<void>}
 */
export async function setupTestEnvironment() {
  // Check server health
  const serverHealthy = await checkServerHealth();
  if (!serverHealthy) {
    throw new Error(`Cannot connect to server at ${SERVER_URL}. Make sure the server is running.`);
  }

  // Build the project
  await buildTxSender();
  
  // Setup required trading pairs
  await setupTradingPairs();
}

/**
 * Helper function to cleanup test environment
 * Resets orderbook state (without recreating pairs for final cleanup)
 * @returns {Promise<void>}
 */
export async function cleanupTestEnvironment() {
  try {
    console.log('🧹 Cleaning up: Resetting orderbook state...');
    await resetOrderbookState(false); // Don't recreate pairs during final cleanup
    console.log('✓ Cleanup completed successfully');
  } catch (error) {
    console.warn('⚠️ Failed to reset orderbook state during cleanup:', error.message);
  }
}