const STORAGE_PREFIX = "hyliquid:";
const ORDERBOOK_TICKS_KEY_PREFIX = `${STORAGE_PREFIX}orderbookTicks:`;
const CHART_OPTIONS_KEY = `${STORAGE_PREFIX}chartDisplayOptions`;
const LEGACY_CHART_STEP_KEY = "chart_step_sec";

export interface ChartDisplayPreferences {
    intervalSeconds: number;
}

const DEFAULT_CHART_DISPLAY_PREFERENCES: ChartDisplayPreferences = Object.freeze({
    intervalSeconds: 3600,
});

const getLocalStorage = (): Storage | null => {
    if (typeof window === "undefined") {
        return null;
    }

    try {
        return window.localStorage;
    } catch (error) {
        console.warn("LocalStorage unavailable:", error);
        return null;
    }
};

const sanitizeIntervalSeconds = (value: unknown): number | null => {
    const parsed = Number(value);
    if (!Number.isFinite(parsed) || parsed <= 0) {
        return null;
    }
    return parsed;
};

export const loadChartPreferences = (): ChartDisplayPreferences => {
    const storage = getLocalStorage();
    if (!storage) {
        return { ...DEFAULT_CHART_DISPLAY_PREFERENCES };
    }

    try {
        const raw = storage.getItem(CHART_OPTIONS_KEY);
        if (raw) {
            const parsed = JSON.parse(raw) as Partial<ChartDisplayPreferences>;
            const intervalSeconds = sanitizeIntervalSeconds(parsed.intervalSeconds);
            if (intervalSeconds) {
                return { intervalSeconds };
            }
        }

        // Fallback to legacy value if present
        const legacyRaw = storage.getItem(LEGACY_CHART_STEP_KEY);
        const legacyInterval = sanitizeIntervalSeconds(legacyRaw);
        if (legacyInterval) {
            const preferences = { intervalSeconds: legacyInterval };
            saveChartPreferences(preferences);
            return preferences;
        }
    } catch (error) {
        console.warn("Failed to load chart preferences:", error);
    }

    return { ...DEFAULT_CHART_DISPLAY_PREFERENCES };
};

export const saveChartPreferences = (preferences: ChartDisplayPreferences): void => {
    const storage = getLocalStorage();
    if (!storage) {
        return;
    }

    const intervalSeconds = sanitizeIntervalSeconds(preferences.intervalSeconds) ?? DEFAULT_CHART_DISPLAY_PREFERENCES.intervalSeconds;
    const payload: ChartDisplayPreferences = { intervalSeconds };

    try {
        storage.setItem(CHART_OPTIONS_KEY, JSON.stringify(payload));
    } catch (error) {
        console.warn("Failed to save chart preferences:", error);
    }
};

export const loadOrderbookTicksPreference = (symbol: string | undefined | null, fallback: number): number => {
    const storage = getLocalStorage();
    if (!storage || !symbol) {
        return fallback;
    }

    try {
        const raw = storage.getItem(`${ORDERBOOK_TICKS_KEY_PREFIX}${symbol}`);
        if (!raw) {
            return fallback;
        }

        const parsed = Number.parseInt(raw, 10);
        if (!Number.isFinite(parsed) || parsed <= 0) {
            return fallback;
        }

        return parsed;
    } catch (error) {
        console.warn("Failed to load orderbook ticks preference:", error);
        return fallback;
    }
};

export const saveOrderbookTicksPreference = (symbol: string | undefined | null, ticks: number): void => {
    const storage = getLocalStorage();
    if (!storage || !symbol) {
        return;
    }

    if (!Number.isFinite(ticks) || ticks <= 0) {
        return;
    }

    try {
        storage.setItem(`${ORDERBOOK_TICKS_KEY_PREFIX}${symbol}`, `${Math.round(ticks)}`);
    } catch (error) {
        console.warn("Failed to save orderbook ticks preference:", error);
    }
};
