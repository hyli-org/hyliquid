<script setup lang="ts">
import type { Order, PaginationInfo } from "../trade";
import { instrumentsState, activityState, nextOrdersPage, prevOrdersPage, changeOrdersSorting } from "../trade";
import { ref } from "vue";

defineProps<{ 
  orders: Order[]; 
  loading?: boolean; 
  error?: string | null;
  pagination?: PaginationInfo | null;
}>();

const pageSizeOptions = [10, 20, 50, 100];
const sortOptions = [
  { value: 'created_at', label: 'Created Date' },
  { value: 'updated_at', label: 'Updated Date' },
  { value: 'price', label: 'Price' },
  { value: 'qty', label: 'Quantity' },
  { value: 'status', label: 'Status' }
];

const sortOrder = ref<'asc' | 'desc'>('desc');

const handleSortChange = async (sortBy: string) => {
  await changeOrdersSorting(sortBy, sortOrder.value);
};

const toggleSortOrder = async () => {
  sortOrder.value = sortOrder.value === 'asc' ? 'desc' : 'asc';
  await changeOrdersSorting(activityState.ordersSortBy, sortOrder.value);
};

const handlePageSizeChange = async (event: Event) => {
  const target = event.target as HTMLSelectElement;
  const newPageSize = parseInt(target.value);
  activityState.ordersPageSize = newPageSize;
  await changeOrdersSorting(activityState.ordersSortBy, activityState.ordersSortOrder);
  // Reset to page 1 when changing page size
  activityState.ordersCurrentPage = 1;
};
</script>

<template>
    <div class="min-h-0 grow flex flex-col rounded-md border border-neutral-800">
        <!-- Controls -->
        <div class="flex items-center justify-between p-3 border-b border-neutral-800 bg-neutral-900/30">
            <div class="flex items-center gap-4">
                <!-- Sort by -->
                <div class="flex items-center gap-2">
                    <label class="text-sm text-neutral-400">Sort by:</label>
                    <select 
                        :value="activityState.ordersSortBy" 
                        @change="handleSortChange(($event.target as HTMLSelectElement).value)"
                        class="px-2 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200"
                    >
                        <option v-for="option in sortOptions" :key="option.value" :value="option.value">
                            {{ option.label }}
                        </option>
                    </select>
                    <button 
                        @click="toggleSortOrder"
                        class="px-2 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200 hover:bg-neutral-700"
                        :title="`Sort ${sortOrder === 'asc' ? 'Ascending' : 'Descending'}`"
                    >
                        {{ sortOrder === 'asc' ? '↑' : '↓' }}
                    </button>
                </div>
                
                <!-- Page size -->
                <div class="flex items-center gap-2">
                    <label class="text-sm text-neutral-400">Per page:</label>
                    <select 
                        :value="activityState.ordersPageSize" 
                        @change="handlePageSizeChange"
                        class="px-2 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200"
                    >
                        <option v-for="size in pageSizeOptions" :key="size" :value="size">
                            {{ size }}
                        </option>
                    </select>
                </div>
            </div>
            
            <!-- Pagination info -->
            <div v-if="pagination" class="text-sm text-neutral-400">
                Showing {{ ((pagination.page - 1) * pagination.limit) + 1 }}-{{ Math.min(pagination.page * pagination.limit, pagination.total) }} of {{ pagination.total }}
            </div>
        </div>
        
        <!-- Table -->
        <div class="flex-1 overflow-auto">
            <table class="w-full text-sm">
                <thead class="sticky top-0 bg-neutral-900/60 text-neutral-400">
                    <tr>
                        <th class="px-3 py-2 text-left font-medium">Symbol</th>
                        <th class="px-3 py-2 text-left font-medium">Side</th>
                        <th class="px-3 py-2 text-left font-medium">Qty</th>
                        <th class="px-3 py-2 text-left font-medium">Qty Remaining</th>
                        <th class="px-3 py-2 text-left font-medium">Price</th>
                        <th class="px-3 py-2 text-left font-medium">Status</th>
                    </tr>
                </thead>
                <tbody>
                    <tr v-if="loading">
                        <td colspan="6" class="px-3 py-3 text-neutral-400">Loading orders…</td>
                    </tr>
                    <tr v-else-if="error">
                        <td colspan="6" class="px-3 py-3 text-rose-400">{{ error }}</td>
                    </tr>
                    <tr v-else-if="orders.length === 0">
                        <td colspan="6" class="px-3 py-3 text-neutral-400">No orders found</td>
                    </tr>
                    <tr v-for="o in orders" :key="o.symbol + o.price + o.type" class="border-t border-neutral-900">
                        <td class="px-3 py-2">{{ o.symbol }}</td>
                        <td class="px-3 py-2" :class="o.side === 'Bid' ? 'text-emerald-400' : 'text-rose-400'">
                            {{ o.side }}
                        </td>
                        <td class="px-3 py-2 tabular-nums">{{ instrumentsState.toRealQty(o.symbol, o.qty) }}</td>
                        <td class="px-3 py-2 tabular-nums">{{ instrumentsState.toRealQty(o.symbol, o.qty_remaining) }}</td>
                        <td class="px-3 py-2 tabular-nums">{{ o.type }} @ {{ o.price ? instrumentsState.toRealPrice(o.symbol, o.price).toLocaleString() : 'market' }}</td>
                        <td class="px-3 py-2">{{ o.status }}</td>
                    </tr>
                </tbody>
            </table>
        </div>
        
        <!-- Pagination controls -->
        <div v-if="pagination && pagination.total_pages > 1" class="flex items-center justify-between p-3 border-t border-neutral-800 bg-neutral-900/30">
            <button 
                @click="prevOrdersPage"
                :disabled="!pagination.has_prev"
                class="px-3 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200 hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed"
            >
                Previous
            </button>
            
            <div class="flex items-center gap-2">
                <span class="text-sm text-neutral-400">
                    Page {{ pagination.page }} of {{ pagination.total_pages }}
                </span>
            </div>
            
            <button 
                @click="nextOrdersPage"
                :disabled="!pagination.has_next"
                class="px-3 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200 hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed"
            >
                Next
            </button>
        </div>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
