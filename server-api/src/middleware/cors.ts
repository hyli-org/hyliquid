/**
 * CORS middleware configuration
 */

import { Elysia } from 'elysia';

export const corsMiddleware = () => {
  return new Elysia({ name: 'cors' })
    .onRequest(({ set }) => {
      // Set CORS headers
      const headers: Record<string, string> = {
        'Access-Control-Allow-Origin': '*',
        'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
        'Access-Control-Allow-Headers': 'Content-Type, Authorization, x-user',
        'Access-Control-Max-Age': '86400', // 24 hours
      };
      
      set.headers = {
        ...set.headers,
        ...headers,
      };
    })
    .options('*', ({ set }) => {
      set.status = 200;
      return '';
    });
};
