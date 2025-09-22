/**
 * Balance endpoints
 */

import { Elysia } from 'elysia';
import { UserService } from '../services';
import { authMiddleware, AuthHeaders } from '../middleware/auth';
import { CustomError } from '../middleware/error-handler';

export const balanceRoutes = (userService: UserService) => {
  return new Elysia({ name: 'balances' })
    .use(authMiddleware())
    .get('/api/balances', async ({ auth }: { auth: AuthHeaders }) => {
      try {
        const balances = await userService.getBalances(auth.user);
        return balances;
      } catch (error) {
        if (error instanceof Error) {
          throw new CustomError(error.message, 404);
        }
        throw error;
      }
    });
};
