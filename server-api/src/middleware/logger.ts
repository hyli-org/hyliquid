/**
 * Logger middleware
 */

import { Elysia } from 'elysia';

export const loggerMiddleware = () => {
  return new Elysia({ name: 'logger' })
    .onRequest(({ request, set }) => {
      // Store start time for elapsed time calculation
      (request as any).startTime = process.hrtime();
    })
      .onAfterHandle(({ request, set }) => {
        const elapsed = process.hrtime((request as any).startTime);
        const elapsedNs = elapsed[0] * 1e9 + elapsed[1]; // Convert to nanoseconds
        const statusCode = set.status || 200;
       
        // Display in microseconds if < 1ms, otherwise in milliseconds
        const elapsedMs = elapsedNs / 1e6;
        const elapsedUs = elapsedNs / 1e3;
        
        if (elapsedMs < 1) {
          console.log(`[${request.method}] ${request.url} - ${statusCode} (${Math.round(elapsedUs)}μs)`);
        } else {
          console.log(`[${request.method}] ${request.url} - ${statusCode} (${elapsedMs.toFixed(2)}ms)`);
        }
      });
};