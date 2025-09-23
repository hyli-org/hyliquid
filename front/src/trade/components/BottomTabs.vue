<script setup lang="ts">
const props = defineProps<{ modelValue: "Positions" | "Orders" | "Fills" }>();
const emit = defineEmits<{ (e: "update:modelValue", v: "Positions" | "Orders" | "Fills"): void }>();
</script>

<template>
    <div class="mb-2 flex gap-2">
        <button
            class="rounded-md px-3 py-1.5 text-sm"
            :class="props.modelValue === 'Positions' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'"
            @click="emit('update:modelValue', 'Positions')"
        >
            Positions
        </button>
        <button
            class="rounded-md px-3 py-1.5 text-sm"
            :class="props.modelValue === 'Orders' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'"
            @click="emit('update:modelValue', 'Orders')"
        >
            Orders
        </button>
        <button
            class="rounded-md px-3 py-1.5 text-sm"
            :class="props.modelValue === 'Fills' ? 'bg-neutral-800 text-white' : 'bg-neutral-900 text-neutral-300'"
            @click="emit('update:modelValue', 'Fills')"
        >
            Fills
        </button>
    </div>
    <slot />
</template>

<style scoped></style>
