/**
 * Error handling middleware
 */

import { Elysia } from 'elysia';

export interface AppError {
  status: number;
  message: string;
}

export class CustomError extends Error {
  public status: number;

  constructor(message: string, status: number = 500) {
    super(message);
    this.status = status;
    this.name = 'CustomError';
  }
}

export const errorHandler = () => {
  return new Elysia({ name: 'error-handler' })
    .onError(({ error, set }) => {
      console.error('Error occurred:', error);

      if (error instanceof CustomError) {
        set.status = error.status;
        return {
          error: error.message,
          status: error.status,
        };
      }

      // Default error response
      set.status = 500;
      return {
        error: 'Internal server error',
        status: 500,
      };
    });
};
