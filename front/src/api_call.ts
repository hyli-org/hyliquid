import { ref, computed, type ComputedRef, watchEffect } from "vue";

export interface UseSWROptions {
    fetchOnMount?: boolean;
}

export type SWRResponse<T> = {
    data: ComputedRef<T | null>;
    error: ComputedRef<any>;
    isLoaded: ComputedRef<boolean>; // Indicates if data has ever been loaded (= data field reflects something), even if have later errors
    fetching: ComputedRef<boolean>;
    revalidate: () => Promise<void>;
    loaded: () => Promise<void>;
};

export function useSWR<T>(fetcher: () => Promise<T>, options: UseSWROptions = {}): SWRResponse<T> {
    const data = ref<T | null>(null);
    const error = ref<any>(null);
    const isLoaded = ref(false); // Indicates if data has ever been loaded (= data field reflects something), even if have later errors
    const fetching = ref(false);

    let lastQuery = 0;

    async function revalidate() {
        let ticket = ++lastQuery;
        fetching.value = true;
        error.value = null;
        try {
            const apiData = await fetcher();
            // Outdated - rudimentary 'cancellation'
            if (ticket !== lastQuery) return;
            data.value = apiData;
            isLoaded.value = true;
        } catch (err) {
            error.value = err;
        }
        fetching.value = false;
    }

    async function loaded() {
        return new Promise<void>((resolve) => {
            const stop = watchEffect(() => {
                if (!fetching.value) {
                    stop();
                    resolve();
                }
            });
        });
    }

    // Fetch on creation if fetchOnMount is true (default: true)
    if (options.fetchOnMount !== false) {
        watchEffect(() => {
            revalidate();
        });
    }

    return {
        data: computed(() => data.value),
        error: computed(() => error.value),
        isLoaded: computed(() => isLoaded.value),
        fetching: computed(() => fetching.value),
        revalidate,
        loaded,
    };
}

export function useApi<T>(url: string, options?: RequestInit): SWRResponse<T> {
    return useSWR(async () => {
        const response = await fetch(url, options);
        if (!response.ok) {
            throw new Error(`Fetch error: ${response.status} ${response.statusText}`);
        }
        return response.json() as Promise<T>;
    });
}
