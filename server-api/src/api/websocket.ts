/**
 * WebSocket status endpoints
 */

import { Elysia } from 'elysia';
import { WebSocketService } from '../services';

export const webSocketRoutes = (webSocketService: WebSocketService) => {
  return new Elysia({ name: 'websocket' })
    .get('/api/websocket/stats', async () => {
      try {
        const stats = webSocketService.getStats();
        return {
          success: true,
          data: stats,
          timestamp: Date.now()
        };
      } catch (error) {
        throw new Error(`Failed to get WebSocket stats: ${error instanceof Error ? error.message : 'Unknown error'}`);
      }
    })
    .get('/api/websocket/health', async () => {
      try {
        const stats = webSocketService.getStats();
        return {
          success: true,
          status: 'healthy',
          connectedClients: stats.connectedClients,
          activeIntervals: stats.activeIntervals,
          totalSubscriptions: stats.subscriptions,
          timestamp: Date.now()
        };
      } catch (error) {
        return {
          success: false,
          status: 'unhealthy',
          error: error instanceof Error ? error.message : 'Unknown error',
          timestamp: Date.now()
        };
      }
    });
};
