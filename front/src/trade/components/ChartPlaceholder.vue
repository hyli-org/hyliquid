<script setup lang="ts">
import { ref, onMounted, onUnmounted, reactive, computed } from 'vue'
import { createChart, CandlestickSeries } from 'lightweight-charts'
import type { IChartApi, ISeriesApi, CandlestickData, Time } from 'lightweight-charts'
import { instrumentsState } from '../trade'
import { loadChartPreferences, saveChartPreferences } from '../preferences'
import { websocketManager } from '../websocket'

const chartContainer = ref<HTMLDivElement>()
let chart: IChartApi | null = null
let candlestickSeries: ISeriesApi<'Candlestick'> | null = null

const chartPreferences = loadChartPreferences()

// Chart state
const chartState = reactive({
    loading: false,
    error: null as string | null,
    data: [] as CandlestickData[],
})

// Get current instrument
const currentInstrument = computed(() => instrumentsState.selected)

// WebSocket candlestick callback
const handleCandlestickUpdate = (candlesticks: any[]) => {
    if (!candlesticks || candlesticks.length === 0) return;

    // Transform WebSocket data to TradingView format
    const transformedData: CandlestickData[] = candlesticks.map((item) => ({
        time: item.time as Time,
        open: instrumentsState.toRealPriceNumber(currentInstrument.value?.symbol, item.open),
        high: instrumentsState.toRealPriceNumber(currentInstrument.value?.symbol, item.high),
        low: instrumentsState.toRealPriceNumber(currentInstrument.value?.symbol, item.low),
        close: instrumentsState.toRealPriceNumber(currentInstrument.value?.symbol, item.close),
    }));

    chartState.data = transformedData;

    // Update chart if it exists
    if (candlestickSeries) {
        candlestickSeries.setData(transformedData);
    }
}

// Function to get bucket time for a given timestamp
function getBucketTime(timestamp: number): number {
    const interval = selectedInterval.value
    if (!interval) return timestamp

    // Round down to the nearest bucket
    const bucketSize = interval.seconds
    return Math.floor(timestamp / bucketSize) * bucketSize
}

// Function to update candlestick data with new trade
function updateCandlestickWithTrade(trade: { price: number; qty: number; time: number }) {
    if (!candlestickSeries || !selectedInterval.value) return

    const tradeTime = trade.time
    const bucketTime = getBucketTime(tradeTime)

    // Get current data
    const currentData = chartState.data

    // Find the last candle
    const lastCandle = currentData[currentData.length - 1]

    if (lastCandle && lastCandle.time === bucketTime) {
        // Update existing candle
        const updatedCandle: CandlestickData = {
            time: bucketTime as Time,
            open: lastCandle.open,
            high: Math.max(lastCandle.high, trade.price),
            low: Math.min(lastCandle.low, trade.price),
            close: trade.price, // Last trade becomes the close price
        }

        // Replace the last candle
        const updatedData = [...currentData.slice(0, -1), updatedCandle]
        chartState.data = updatedData
        candlestickSeries.setData(updatedData)
    } else {
        // Create new candle
        const newCandle: CandlestickData = {
            time: bucketTime as Time,
            open: trade.price,
            high: trade.price,
            low: trade.price,
            close: trade.price,
        }

        // Add new candle
        const updatedData = [...currentData, newCandle]
        chartState.data = updatedData
        candlestickSeries.setData(updatedData)
    }
}

// Subscribe to candlestick updates via WebSocket
function subscribeToCandlestick() {
    if (currentInstrument.value) {
        websocketManager.subscribeToCandlestick(
            currentInstrument.value.symbol,
            chartPreferences.intervalSeconds
        );
    }
}

// Unsubscribe from candlestick updates
function unsubscribeFromCandlestick() {
    websocketManager.unsubscribeCandlestick();
}

// Computed property to check if interval is selected
const isIntervalSelected = (interval: typeof timeIntervals[0]) => {
    return selectedInterval.value?.seconds === interval.seconds
}

// Predefined time intervals in seconds
const timeIntervals = [
    { label: '1m', seconds: 60 },
    { label: '3m', seconds: 180 },
    { label: '5m', seconds: 300 },
    { label: '15m', seconds: 900 },
    { label: '30m', seconds: 1800 },
    { label: '1h', seconds: 3600 },
    { label: '2h', seconds: 7200 },
    { label: '4h', seconds: 14400 },
    { label: '8h', seconds: 28800 },
    { label: '12h', seconds: 43200 },
    { label: '1d', seconds: 86400 },
    { label: '3d', seconds: 259200 },
    { label: '1M', seconds: 2592000 }, // Approximate month
]

// Selected interval
const matchedInterval = timeIntervals.find(interval => interval.seconds === chartPreferences.intervalSeconds)
const selectedInterval = ref<typeof timeIntervals[0]>(matchedInterval ?? timeIntervals[5]!) // Default to 1h

if (!matchedInterval) {
    chartPreferences.intervalSeconds = selectedInterval.value.seconds
    saveChartPreferences(chartPreferences)
}

// Function to update time scale configuration based on interval
function updateTimeScaleConfig() {
    if (!chart) return

    const interval = selectedInterval.value
    if (!interval) return

    // Configure time scale based on interval
    let timeScaleConfig: any = {
        timeVisible: true,
        secondsVisible: false,
        rightOffset: 12,
        barSpacing: 6,
        minBarSpacing: 0.5,
        lockVisibleTimeRangeOnResize: true,
        rightBarStaysOnScroll: true,
        shiftVisibleRangeOnNewBar: true,
        tickMarkFormatter: (time: number, _tickMarkType: any, locale: string) => {
            const date = new Date(time * 1000)

            // Format based on interval
            if (interval.seconds <= 60) { // 1 minute or less
                return date.toLocaleTimeString(locale, {
                    hour: '2-digit',
                    minute: '2-digit',
                    second: '2-digit'
                })
            } else if (interval.seconds <= 3600) { // 1 hour or less
                return date.toLocaleTimeString(locale, {
                    hour: '2-digit',
                    minute: '2-digit'
                })
            } else if (interval.seconds <= 86400) { // 1 day or less
                return date.toLocaleDateString(locale, {
                    month: 'short',
                    day: 'numeric',
                    hour: '2-digit'
                })
            } else { // More than 1 day
                return date.toLocaleDateString(locale, {
                    month: 'short',
                    day: 'numeric'
                })
            }
        }
    }

    // Adjust bar spacing and visibility based on interval
    if (interval.seconds <= 60) { // 1 minute or less
        timeScaleConfig.barSpacing = 3
        timeScaleConfig.minBarSpacing = 0.5
        timeScaleConfig.secondsVisible = true
    } else if (interval.seconds <= 300) { // 5 minutes or less
        timeScaleConfig.barSpacing = 4
        timeScaleConfig.minBarSpacing = 0.5
    } else if (interval.seconds <= 3600) { // 1 hour or less
        timeScaleConfig.barSpacing = 6
        timeScaleConfig.minBarSpacing = 1
    } else if (interval.seconds <= 86400) { // 1 day or less
        timeScaleConfig.barSpacing = 8
        timeScaleConfig.minBarSpacing = 2
    } else { // More than 1 day
        timeScaleConfig.barSpacing = 10
        timeScaleConfig.minBarSpacing = 3
    }

    chart.applyOptions({
        timeScale: timeScaleConfig
    })

    chart.timeScale().fitContent()
}

// Function to update step interval
function updateStepInterval(interval: typeof timeIntervals[0]) {
    selectedInterval.value = interval
    chartPreferences.intervalSeconds = interval.seconds
    saveChartPreferences(chartPreferences)

    // Resubscribe to candlestick with new interval
    unsubscribeFromCandlestick()
    subscribeToCandlestick()
}

// Function to manually add a trade (useful for testing)
function addTrade(price: number, qty: number, time?: number) {
    const tradeTime = time || Math.floor(Date.now() / 1000)
    updateCandlestickWithTrade({ price, qty, time: tradeTime })
}

// Expose functions and state for parent components
defineExpose({
    chartState,
    currentInstrument,
    timeIntervals,
    selectedInterval,
    updateStepInterval,
    updateTimeScaleConfig,
    updateCandlestickWithTrade,
    subscribeToCandlestick,
    unsubscribeFromCandlestick,
    addTrade,
})

onMounted(async () => {
    if (!chartContainer.value) return

    // Create chart
    chart = createChart(chartContainer.value, {
        width: chartContainer.value.clientWidth,
        height: chartContainer.value.clientHeight,
        layout: {
            background: { color: 'transparent' },
            textColor: '#9CA3AF',
        },
        grid: {
            vertLines: { color: '#374151' },
            horzLines: { color: '#374151' },
        },
        crosshair: {
            mode: 1,
        },
        rightPriceScale: {
            borderColor: '#374151',
            textColor: '#9CA3AF',
        },
        timeScale: {
            borderColor: '#374151',
            timeVisible: true,
            secondsVisible: false,
            rightOffset: 12,
            barSpacing: 6,
            minBarSpacing: 0.5,
            lockVisibleTimeRangeOnResize: true,
            rightBarStaysOnScroll: true,
            shiftVisibleRangeOnNewBar: true,
        },
    })

    // Add candlestick series
    candlestickSeries = chart.addSeries(CandlestickSeries, {
        upColor: '#10B981',
        downColor: '#EF4444',
        borderVisible: false,
        wickUpColor: '#10B981',
        wickDownColor: '#EF4444',
    })

    // Subscribe to WebSocket candlestick updates
    websocketManager.onCandlestickUpdate(handleCandlestickUpdate)
    subscribeToCandlestick()

    // Configure time scale based on current interval
    updateTimeScaleConfig()

    // Handle resize
    const handleResize = () => {
        if (chart && chartContainer.value) {
            chart.applyOptions({
                width: chartContainer.value.clientWidth,
                height: chartContainer.value.clientHeight,
            })
        }
    }

    window.addEventListener('resize', handleResize)

    // Cleanup function
    onUnmounted(() => {
        window.removeEventListener('resize', handleResize)
        websocketManager.offCandlestickUpdate(handleCandlestickUpdate)
        unsubscribeFromCandlestick()
        if (chart) {
            chart.remove()
        }
    })
})
</script>

<template>
    <div class="h-96 border-b border-neutral-800 p-4">
        <!-- Chart Controls -->
        <div class="flex items-center justify-between mb-4">
            <div class="flex items-center space-x-2">
                <span class="text-sm text-neutral-400">Timeframe:</span>
                <div class="flex items-center space-x-1 bg-neutral-800 rounded-md p-1">
                    <button v-for="interval in timeIntervals" :key="interval.seconds"
                        @click="updateStepInterval(interval)" :class="[
                            'px-2 py-1 text-xs rounded transition-colors',
                            isIntervalSelected(interval)
                                ? 'bg-neutral-600 text-white'
                                : 'text-neutral-400 hover:text-neutral-200 hover:bg-neutral-700'
                        ]">
                        {{ interval.label }}
                    </button>
                </div>
            </div>

            <!-- Chart Info -->
            <div class="text-xs text-neutral-500" v-if="chartState.data.length > 0">
                {{ chartState.data.length }} candles
            </div>
        </div>

        <div ref="chartContainer" class="h-80 w-full rounded-md border border-neutral-800 bg-neutral-900/30 relative">
            <!-- Loading state -->
            <div v-if="chartState.loading"
                class="absolute inset-0 flex items-center justify-center bg-neutral-900/50 rounded-md">
                <div class="flex items-center space-x-2 text-neutral-400">
                    <div class="animate-spin rounded-full h-4 w-4 border-b-2 border-neutral-400"></div>
                    <span>Loading chart data...</span>
                </div>
            </div>

            <!-- Error state -->
            <div v-if="chartState.error"
                class="absolute inset-0 flex items-center justify-center bg-neutral-900/50 rounded-md">
                <div class="text-center text-neutral-400">
                    <div class="text-red-400 mb-2">⚠️ Error loading chart</div>
                    <div class="text-sm mb-4">{{ chartState.error }}</div>
                </div>
            </div>

            <!-- Empty state -->
            <div v-if="!chartState.loading && !chartState.error && chartState.data.length === 0"
                class="absolute inset-0 flex items-center justify-center bg-neutral-900/50 rounded-md">
                <div class="text-center text-neutral-500">
                    <div class="mb-2">📊 No chart data available</div>
                    <div class="text-sm">No trading data found for the selected time period</div>
                </div>
            </div>
        </div>
    </div>
</template>

<style scoped></style>
