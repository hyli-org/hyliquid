<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";

const props = defineProps<{ modelValue: "Positions" | "Orders" | "Fills" | "Balances" }>();
const emit = defineEmits<{ (e: "update:modelValue", v: "Positions" | "Orders" | "Fills" | "Balances"): void }>();

// Map tab names to hash values
const tabToHash: Record<"Positions" | "Orders" | "Fills" | "Balances", string> = {
    Positions: "#positions",
    Orders: "#orders",
    Fills: "#fills",
    Balances: "#balances",
};

// Map hash values to tab names
const hashToTab: Record<string, "Positions" | "Orders" | "Fills" | "Balances"> = {
    "#positions": "Positions",
    "#orders": "Orders",
    "#fills": "Fills",
    "#balances": "Balances",
};

// Function to update URL hash
const updateHash = (tab: "Positions" | "Orders" | "Fills" | "Balances") => {
    const hash = tabToHash[tab];
    if (window.location.hash !== hash) {
        window.location.hash = hash;
    }
};

// Function to handle tab click
const handleTabClick = (tab: "Positions" | "Orders" | "Fills" | "Balances") => {
    emit("update:modelValue", tab);
    updateHash(tab);
};

// Function to handle hash change
const handleHashChange = () => {
    const hash = window.location.hash;
    const tab = hashToTab[hash];
    if (tab && tab !== props.modelValue) {
        emit("update:modelValue", tab);
    }
};

onMounted(() => {
    // Listen for hash changes
    window.addEventListener("hashchange", handleHashChange);

    // Initialize from current hash if present
    const currentHash = window.location.hash;
    const tab = hashToTab[currentHash];
    if (tab && tab !== props.modelValue) {
        emit("update:modelValue", tab);
    }
});

onUnmounted(() => {
    window.removeEventListener("hashchange", handleHashChange);
});
</script>

<template>
    <div class="mb-2 flex gap-2">
        <button
            class="rounded-md px-3 py-1.5 text-sm transition"
            :class="
                props.modelValue === 'Positions'
                    ? 'border border-[var(--border-accent)] bg-[var(--accent-soft)] text-[var(--text-accent)] shadow-sm'
                    : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            "
            @click="handleTabClick('Positions')"
        >
            Positions
        </button>
        <button
            class="rounded-md px-3 py-1.5 text-sm transition"
            :class="
                props.modelValue === 'Orders'
                    ? 'border border-[var(--border-accent)] bg-[var(--accent-soft)] text-[var(--text-accent)] shadow-sm'
                    : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            "
            @click="handleTabClick('Orders')"
        >
            Orders
        </button>
        <button
            class="rounded-md px-3 py-1.5 text-sm transition"
            :class="
                props.modelValue === 'Fills'
                    ? 'border border-[var(--border-accent)] bg-[var(--accent-soft)] text-[var(--text-accent)] shadow-sm'
                    : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            "
            @click="handleTabClick('Fills')"
        >
            Fills
        </button>
        <button
            class="rounded-md px-3 py-1.5 text-sm transition"
            :class="
                props.modelValue === 'Balances'
                    ? 'border border-[var(--border-accent)] bg-[var(--accent-soft)] text-[var(--text-accent)] shadow-sm'
                    : 'border border-[var(--border-default)] bg-[var(--surface-input)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            "
            @click="handleTabClick('Balances')"
        >
            Balances
        </button>
    </div>
    <slot />
</template>

<style scoped></style>
