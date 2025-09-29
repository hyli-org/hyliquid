/**
 * Error handling middleware
 */

import { Elysia } from "elysia";
import { log_request } from "./logger";

export interface AppError {
  status: number;
  message: string;
}

export class CustomError extends Error {
  public status: number;

  constructor(message: string, status: number = 500) {
    super(message);
    this.status = status;
    this.name = "CustomError";
  }
}

export const errorHandler = () => {
  return new Elysia({ name: "error-handler" }).onError(
    ({ request, error, set }) => {
      if (error instanceof CustomError) {
        log_request(request, error.status, error.message);
        set.status = error.status;
        return {
          error: error.message,
          status: error.status,
        };
      }
      log_request(request, 500, error.message);

      // Default error response
      set.status = 500;
      return {
        error: "Internal server error",
        status: 500,
      };
    }
  );
};
