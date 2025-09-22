/**
 * Health check endpoint
 */

import { Elysia } from 'elysia';

export const healthRoutes = () => {
  return new Elysia({ name: 'health' })
    .get('/_health', () => {
      return 'OK';
    });
};
