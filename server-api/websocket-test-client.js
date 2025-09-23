/**
 * Simple WebSocket test client for testing the l2book channel
 * Run this after starting the server to test WebSocket functionality
 */

const WebSocket = require('ws');

const WS_URL = 'ws://localhost:3000/ws';
let ws;

// Read from command line arguments or default values
const args = process.argv.slice(2);
let instrument = 'hyllar/oranj' ;
let groupTicks = 10;
if (args.length > 0) {
  instrument = args[0];
}
if (args.length > 1) {
  groupTicks = parseInt(args[1]);
}

function connect() {
  ws = new WebSocket(WS_URL);
  
  ws.on('open', () => {
    console.log('✅ Connected to WebSocket server');
    
    // Subscribe to instrument l2book with groupTicks
    const subscribeMessage = {
      method: 'subscribe',
      subscription: {
        type: 'l2Book',
        instrument: instrument,
        groupTicks: groupTicks
      }
    };
    
    console.log('📤 Sending subscribe message:', JSON.stringify(subscribeMessage, null, 2));
    ws.send(JSON.stringify(subscribeMessage));
  });
  
  ws.on('message', (data) => {
    try {
      const message = JSON.parse(data.toString());
      console.log('📥 Received message:', JSON.stringify(message, null, 2));
    } catch (error) {
      console.error('❌ Error parsing message:', error);
    }
  });
  
  ws.on('close', () => {
    console.log('🔌 WebSocket connection closed');
    process.exit(0);
  });
  
  ws.on('error', (error) => {
    console.error('❌ WebSocket error:', error);
  });
}

// Handle graceful shutdown
process.on('SIGINT', () => {
  console.log('\n👋 Shutting down test client...');
  if (ws) {
    ws.close();
  }
  process.exit(0);
});

console.log('🚀 Starting WebSocket test client...');
console.log(`🔗 Connecting to: ${WS_URL}`);
console.log('📋 This will:');
console.log('   1. Connect to WebSocket server');
console.log('   2. Subscribe to ' + instrument + ' l2book with groupTicks=' + groupTicks);
console.log('   3. Receive updates every 1 second');
console.log('   4. Continue receiving updates with new groupTicks');
console.log('\nPress Ctrl+C to exit\n');

connect();

// Keep the process alive
setInterval(() => {
  // This interval keeps the event loop running
  // The actual work is done by the WebSocket event handlers
}, 1000);
