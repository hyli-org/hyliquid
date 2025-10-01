import { ref, watchEffect } from "vue";

export interface UseSWROptions {
    fetchOnMount?: boolean;
}

export class SWRQuery<T> {
    data = ref<T | null>(null);
    error = ref<any>(null);
    isLoaded = ref(false); // Indicates if data has ever been loaded (= data field reflects something), even if have later errors
    fetching = ref(false);

    private fetcher: () => Promise<T>;
    private lastQuery = 0;

    constructor(fetcher: () => Promise<T>, options: UseSWROptions = { fetchOnMount: true }) {
        this.fetcher = fetcher;
        // Fetch on creation if fetchOnMount is true (default: true)
        if (options.fetchOnMount === true) {
            this.revalidate();
        }
    }

    async watch() {
        watchEffect(() => {
            this.revalidate();
        });
        return this;
    }

    async revalidate() {
        let ticket = ++this.lastQuery;
        this.fetching.value = true;
        this.error.value = null;
        try {
            const apiData = await this.fetcher();
            // Outdated - rudimentary 'cancellation'
            if (ticket !== this.lastQuery) return;
            this.data.value = apiData;
            this.isLoaded.value = true;
        } catch (err) {
            this.error.value = err;
        }
        this.fetching.value = false;
    }

    async loaded() {
        return new Promise<void>((resolve) => {
            const stop = watchEffect(() => {
                if (!this.fetching.value && (this.isLoaded.value || this.error.value)) {
                    stop();
                    resolve();
                }
            });
        });
    }
}

// Convenience wrapper
export function useSWR<T>(fetcher: () => Promise<T>, options: UseSWROptions = { fetchOnMount: true }): SWRQuery<T> {
    return new SWRQuery<T>(fetcher, options);
}

export function useApi<T>(url: string, options?: RequestInit): SWRQuery<T> {
    return useSWR(async () => {
        const response = await fetch(url, options);
        if (!response.ok) {
            throw new Error(`Fetch error: ${response.status} ${response.statusText}`);
        }
        return response.json() as Promise<T>;
    });
}
