/**
 * Application configuration
 */

export interface AppConfig {
  port: number;
  host: string;
  contractName: string;
  nodeEnv: string;
}

export function getAppConfig(): AppConfig {
  return {
    port: parseInt(process.env.PORT || '3000', 10),
    host: process.env.HOST || '0.0.0.0',
    contractName: process.env.CONTRACT_NAME || 'orderbook',
    nodeEnv: process.env.NODE_ENV || 'development',
  };
}
