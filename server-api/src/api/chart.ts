/**
 * Chart data API routes
 */

import { Elysia } from "elysia";
import { DatabaseQueries } from "../database/queries";
import { CandlestickData } from "../types/api";

export const chartRoutes = (dbQueries: DatabaseQueries) => {
  return new Elysia({ prefix: "/api/chart" }).get(
    "/candlestick",
    async ({ query, set }) => {
      try {
        const { instrument_id, t_from, t_to, step_sec } = query as {
          instrument_id?: string;
          t_from?: string;
          t_to?: string;
          step_sec?: string;
        };

        // Validate required parameters
        if (!instrument_id || !t_from || !t_to || !step_sec) {
          set.status = 400;
          return {
            error:
              "Missing required parameters: instrument_id, t_from, t_to, step_sec",
          };
        }

        const instrumentId = parseInt(instrument_id, 10);
        const stepSec = parseInt(step_sec, 10);

        // Validate parameter types
        if (isNaN(instrumentId) || isNaN(stepSec)) {
          set.status = 400;
          return {
            error:
              "Invalid parameter types: instrument_id and step_sec must be numbers",
          };
        }

        // Validate date formats
        const tFromDate = new Date(t_from);
        const tToDate = new Date(t_to);

        if (isNaN(tFromDate.getTime()) || isNaN(tToDate.getTime())) {
          set.status = 400;
          return {
            error:
              "Invalid date format: t_from and t_to must be valid ISO timestamps",
          };
        }

        // Validate time range
        if (tFromDate >= tToDate) {
          set.status = 400;
          return {
            error: "Invalid time range: t_from must be before t_to",
          };
        }

        // Get candlestick data from database
        const rawData = await dbQueries.getCandlestickData(
          instrumentId,
          t_from,
          t_to,
          stepSec
        );

        // Transform data to match TradingView format
        const candlestickData: CandlestickData[] = rawData.map((row) => ({
          time: Math.floor(new Date(row.bucket).getTime() / 1000), // Convert to Unix timestamp
          open: row.open,
          high: row.high,
          low: row.low,
          close: row.close,
          volume_trades: row.volume_trades,
          trade_count: row.trade_count,
        }));

        return {
          data: candlestickData,
          metadata: {
            instrument_id: instrumentId,
            t_from,
            t_to,
            step_sec: stepSec,
            count: candlestickData.length,
          },
        };
      } catch (error) {
        console.error("Error fetching candlestick data:", error);
        set.status = 500;
        return {
          error: "Internal server error while fetching candlestick data",
        };
      }
    }
  );
};
