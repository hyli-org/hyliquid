/**
 * Authentication middleware
 */

import { Elysia } from 'elysia';
import { CustomError } from './error-handler';

export interface AuthHeaders {
  user: string;
}

export const authMiddleware = () => {
  return new Elysia({ name: 'auth' })
    .derive(({ headers }) => {
      const user = headers['x-user'] || headers['X-User'];
      
      if (!user || typeof user !== 'string') {
        throw new CustomError('Missing x-user header', 401);
      }

      return {
        auth: {
          user,
        } as AuthHeaders,
      };
    });
};

export const optionalAuthMiddleware = () => {
  return new Elysia({ name: 'optional-auth' })
    .derive(({ headers }) => {
      const user = headers['x-user'] || headers['X-User'];
      
      return {
        auth: user && typeof user === 'string' ? { user } as AuthHeaders : null,
      };
    });
};
