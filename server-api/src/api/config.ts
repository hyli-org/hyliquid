/**
 * Configuration endpoint
 */

import { Elysia } from 'elysia';
import { ConfigResponse } from '../types';
import { getAppConfig } from '../config';

export const configRoutes = () => {
  const config = getAppConfig();

  return new Elysia({ name: 'config' })
    .get('/api/config', (): ConfigResponse => {
      return {
        contract_name: config.contractName,
      };
    });
};
