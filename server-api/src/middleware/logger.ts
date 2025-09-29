/**
 * Logger middleware
 */

import { Elysia } from "elysia";

export const log_request = (
  request: any,
  statusCode: number,
  extras: string = ""
) => {
  const elapsed = process.hrtime((request as any).startTime);
  const elapsedNs = elapsed[0] * 1e9 + elapsed[1]; // Convert to nanoseconds

  // Display in microseconds if < 1ms, otherwise in milliseconds
  const elapsedMs = elapsedNs / 1e6;
  const elapsedUs = elapsedNs / 1e3;

  if (elapsedMs < 1) {
    console.log(
      `[${request.method}] ${request.url} - ${statusCode} (${Math.round(
        elapsedUs
      )}μs) ${extras}`
    );
  } else {
    console.log(
      `[${request.method}] ${request.url} - ${statusCode} (${elapsedMs.toFixed(
        2
      )}ms) ${extras}`
    );
  }
};

export const loggerMiddleware = () => {
  return new Elysia({ name: "logger" })
    .onRequest(({ request, set }) => {
      // Store start time for elapsed time calculation
      (request as any).startTime = process.hrtime();
    })
    .onAfterHandle(({ request, set }) => {
      log_request(request, (set.status as number) || 200);
    });
};
