/**
 * Utility functions for extracting route information from Elysia app
 */

import { Elysia } from 'elysia';

export interface RouteInfo {
  method: string;
  path: string;
}

/**
 * Extracts route information from an Elysia app instance
 * This function traverses the app's route tree and extracts method/path combinations
 */
export function extractRouteInfo(app: Elysia): RouteInfo[] {
  const routes: RouteInfo[] = [];
  
  // Get the internal route tree from the Elysia instance
  const routeTree = (app as any).routes || [];
  
  function traverseRoutes(routeList: any[], prefix: string = '') {
    for (const route of routeList) {
      if (route.method && route.path) {
        // Add the route with proper method formatting
        routes.push({
          method: route.method.toUpperCase(),
          path: prefix + route.path,
        });
      }
      
      // Handle nested routes
      if (route.routes) {
        const newPrefix = prefix + (route.path || '');
        traverseRoutes(route.routes, newPrefix);
      }
    }
  }
  
  traverseRoutes(routeTree);
  
  return routes;
}

/**
 * Formats and displays route information in a readable format
 */
export function displayRoutes(routes: RouteInfo[]): void {
  if (routes.length === 0) {
    console.log('No routes found');
    return;
  }
  
  console.log('Available endpoints:');
  
  // Sort routes by path for better readability
  const sortedRoutes = routes.sort((a, b) => {
    // First sort by method, then by path
    if (a.method !== b.method) {
      return a.method.localeCompare(b.method);
    }
    return a.path.localeCompare(b.path);
  });
  
  for (const route of sortedRoutes) {
    const method = route.method.padEnd(6);
    console.log(`  ${method} ${route.path}`);
  }
}
