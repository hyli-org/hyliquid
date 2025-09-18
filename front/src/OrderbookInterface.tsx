import { useState, useEffect } from 'react';
import './OrderbookInterface.css';

interface Order {
  price: number;
  amount: number;
  total: number;
  percentage?: number;
}

interface OrderbookData {
  bids: Order[];
  asks: Order[];
  lastPrice: number | null;
  change24h: number | null;
  volume24h: number | null;
  high24h: number | null;
  low24h: number | null;
}

interface RecentTrade {
  price: number;
  amount: number;
  side: 'buy' | 'sell';
  timestamp: string;
}

// API configuration
const API_CONFIG = {
  baseUrl: import.meta.env.VITE_API_BASE_URL || 'http://localhost:8000',
  endpoints: {
    orderbook: '/api/orderbook',
    ticker: '/api/ticker',
    trades: '/api/trades',
    balance: '/api/balance'
  },
  refreshInterval: 1000 // 1 second
};

// Mock data generation functions
const generateMockOrderbook = (): { bids: Order[], asks: Order[] } => {
  const basePrice = 45000;
  const bids: Order[] = [];
  const asks: Order[] = [];

  // Generate bids (buy orders)
  for (let i = 0; i < 15; i++) {
    const price = basePrice - (i * 10) - Math.random() * 10;
    const amount = Math.random() * 2 + 0.01;
    bids.push({
      price: parseFloat(price.toFixed(2)),
      amount: parseFloat(amount.toFixed(8)),
      total: parseFloat((price * amount).toFixed(2)),
      percentage: Math.random() * 100
    });
  }

  // Generate asks (sell orders)
  for (let i = 0; i < 15; i++) {
    const price = basePrice + (i * 10) + Math.random() * 10;
    const amount = Math.random() * 2 + 0.01;
    asks.push({
      price: parseFloat(price.toFixed(2)),
      amount: parseFloat(amount.toFixed(8)),
      total: parseFloat((price * amount).toFixed(2)),
      percentage: Math.random() * 100
    });
  }

  return { bids, asks };
};

const generateMockTrades = (): RecentTrade[] => {
  const trades: RecentTrade[] = [];
  const basePrice = 45000;

  for (let i = 0; i < 20; i++) {
    const price = basePrice + (Math.random() - 0.5) * 200;
    const amount = Math.random() * 0.5 + 0.001;
    const side = Math.random() > 0.5 ? 'buy' : 'sell';
    const now = new Date();
    now.setSeconds(now.getSeconds() - i * 30);

    trades.push({
      price: parseFloat(price.toFixed(2)),
      amount: parseFloat(amount.toFixed(8)),
      side: side as 'buy' | 'sell',
      timestamp: now.toLocaleTimeString()
    });
  }

  return trades;
};

interface OrderbookInterfaceProps {
  mockMode: boolean;
}

function OrderbookInterface({ mockMode }: OrderbookInterfaceProps) {
  const [selectedPair, setSelectedPair] = useState('BTC/USDC');
  const [orderType, setOrderType] = useState<'limit' | 'market'>('limit');
  const [orderSide, setOrderSide] = useState<'buy' | 'sell'>('buy');
  const [price, setPrice] = useState('');
  const [amount, setAmount] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Initialize with empty/null data
  const [orderbookData, setOrderbookData] = useState<OrderbookData>({
    bids: [],
    asks: [],
    lastPrice: null,
    change24h: null,
    volume24h: null,
    high24h: null,
    low24h: null
  });

  const [recentTrades, setRecentTrades] = useState<RecentTrade[]>([]);
  const [balance, setBalance] = useState({
    usdc: null as number | null,
    btc: null as number | null
  });

  // Fetch orderbook data from API or generate mock data
  const fetchOrderbookData = async () => {
    try {
      if (mockMode) {
        // Generate mock data
        const { bids, asks } = generateMockOrderbook();
        const lastPrice = 45000 + (Math.random() - 0.5) * 100;
        const change24h = (Math.random() - 0.5) * 10;
        const volume24h = Math.random() * 10000000 + 1000000;
        const high24h = lastPrice + Math.random() * 500;
        const low24h = lastPrice - Math.random() * 500;

        setOrderbookData({
          bids,
          asks,
          lastPrice,
          change24h,
          volume24h,
          high24h,
          low24h
        });
      } else {
        // Clear mock data when mock mode is off
        setOrderbookData({
          bids: [],
          asks: [],
          lastPrice: null,
          change24h: null,
          volume24h: null,
          high24h: null,
          low24h: null
        });
        // TODO: Replace with actual API calls
        // const response = await fetch(`${API_CONFIG.baseUrl}${API_CONFIG.endpoints.orderbook}/${selectedPair}`);
        // const data = await response.json();
        // setOrderbookData(data);
      }
      setLoading(false);
    } catch (err) {
      setError('Failed to fetch orderbook data');
      setLoading(false);
    }
  };

  // Fetch ticker data from API
  const fetchTickerData = async () => {
    try {
      // TODO: Replace with actual API call
      // const response = await fetch(`${API_CONFIG.baseUrl}${API_CONFIG.endpoints.ticker}/${selectedPair}`);
      // const data = await response.json();
      // Update orderbookData with ticker info
    } catch (err) {
      console.error('Failed to fetch ticker data:', err);
    }
  };

  // Fetch recent trades from API or generate mock data
  const fetchRecentTrades = async () => {
    try {
      if (mockMode) {
        setRecentTrades(generateMockTrades());
      } else {
        // Clear mock data when mock mode is off
        setRecentTrades([]);
        // TODO: Replace with actual API call
        // const response = await fetch(`${API_CONFIG.baseUrl}${API_CONFIG.endpoints.trades}/${selectedPair}`);
        // const data = await response.json();
        // setRecentTrades(data);
      }
    } catch (err) {
      console.error('Failed to fetch recent trades:', err);
    }
  };

  // Fetch user balance from API or generate mock data
  const fetchBalance = async () => {
    try {
      if (mockMode) {
        setBalance({
          usdc: 10000.50,
          btc: 0.25342187
        });
      } else {
        // Clear mock data when mock mode is off
        setBalance({
          usdc: null,
          btc: null
        });
        // TODO: Replace with actual API call
        // const response = await fetch(`${API_CONFIG.baseUrl}${API_CONFIG.endpoints.balance}`);
        // const data = await response.json();
        // setBalance(data);
      }
    } catch (err) {
      console.error('Failed to fetch balance:', err);
    }
  };

  useEffect(() => {
    // Initial data fetch
    fetchOrderbookData();
    fetchTickerData();
    fetchRecentTrades();
    fetchBalance();

    // Set up polling for real-time updates
    const interval = setInterval(() => {
      fetchOrderbookData();
      fetchTickerData();
      fetchRecentTrades();
    }, API_CONFIG.refreshInterval);

    return () => clearInterval(interval);
  }, [selectedPair, mockMode]);

  const formatNumber = (num: number | null, decimals: number = 2): string => {
    if (num === null || num === undefined) return 'N/A';
    if (num >= 1e9) return `${(num / 1e9).toFixed(2)}B`;
    if (num >= 1e6) return `${(num / 1e6).toFixed(2)}M`;
    if (num >= 1e3) return `${(num / 1e3).toFixed(2)}K`;
    return num.toFixed(decimals);
  };

  const formatPrice = (price: number | null): string => {
    if (price === null || price === undefined) return 'N/A';
    return price.toFixed(2);
  };

  const formatAmount = (amount: number | null): string => {
    if (amount === null || amount === undefined) return 'N/A';
    return amount.toFixed(8);
  };

  const handlePriceClick = (clickedPrice: number) => {
    setPrice(clickedPrice.toString());
  };

  const handleAmountClick = (clickedAmount: number) => {
    setAmount(clickedAmount.toString());
  };

  const handleSubmitOrder = async () => {
    // TODO: Implement order submission
    console.log('Submitting order:', { orderSide, orderType, price, amount });
  };

  const calculateSpread = (): { value: string; percentage: string } => {
    if (orderbookData.asks.length === 0 || orderbookData.bids.length === 0) {
      return { value: 'N/A', percentage: 'N/A' };
    }

    const spread = orderbookData.asks[0].price - orderbookData.bids[0].price;
    const spreadPercentage = orderbookData.lastPrice
      ? (spread / orderbookData.lastPrice * 100).toFixed(3)
      : 'N/A';

    return {
      value: spread.toFixed(2),
      percentage: spreadPercentage !== 'N/A' ? `${spreadPercentage}%` : 'N/A'
    };
  };

  const spread = calculateSpread();

  return (
    <div className="orderbook-container">
      <div className="orderbook-header">
        <div className="pair-selector">
          <select
            value={selectedPair}
            onChange={(e) => setSelectedPair(e.target.value)}
            className="pair-dropdown"
          >
            <option value="BTC/USDC">BTC/USDC</option>
            <option value="BTC/USDT">BTC/USDT</option>
          </select>
        </div>

        <div className="market-stats">
          <div className="stat">
            <span className="stat-label">Last Price</span>
            <span className="stat-value price-value">
              {orderbookData.lastPrice !== null ? `$${formatNumber(orderbookData.lastPrice)}` : 'N/A'}
            </span>
          </div>
          <div className="stat">
            <span className="stat-label">24h Change</span>
            <span className={`stat-value ${orderbookData.change24h !== null ? (orderbookData.change24h >= 0 ? 'positive' : 'negative') : ''}`}>
              {orderbookData.change24h !== null
                ? `${orderbookData.change24h >= 0 ? '+' : ''}${orderbookData.change24h.toFixed(2)}%`
                : 'N/A'}
            </span>
          </div>
          <div className="stat">
            <span className="stat-label">24h Volume</span>
            <span className="stat-value">
              {orderbookData.volume24h !== null ? `$${formatNumber(orderbookData.volume24h)}` : 'N/A'}
            </span>
          </div>
          <div className="stat">
            <span className="stat-label">24h High</span>
            <span className="stat-value">
              {orderbookData.high24h !== null ? `$${formatNumber(orderbookData.high24h)}` : 'N/A'}
            </span>
          </div>
          <div className="stat">
            <span className="stat-label">24h Low</span>
            <span className="stat-value">
              {orderbookData.low24h !== null ? `$${formatNumber(orderbookData.low24h)}` : 'N/A'}
            </span>
          </div>
        </div>
      </div>

      <div className="orderbook-main">
        <div className="orderbook-left">
          <div className="orderbook-panel">
            <div className="orderbook-title">Order Book</div>

            <div className="orderbook-table">
              <div className="table-header">
                <span>Price (USDC)</span>
                <span>Amount (BTC)</span>
                <span>Total (USDC)</span>
              </div>

              <div className="asks-section">
                {orderbookData.asks.length > 0 ? (
                  orderbookData.asks.slice().reverse().map((ask, index) => (
                    <div
                      key={index}
                      className="order-row ask"
                      onClick={() => handlePriceClick(ask.price)}
                    >
                      <div
                        className="depth-bar ask-depth"
                        style={{ width: `${ask.percentage || 0}%` }}
                      />
                      <span className="price ask-price">{formatPrice(ask.price)}</span>
                      <span className="amount" onClick={(e) => {
                        e.stopPropagation();
                        handleAmountClick(ask.amount);
                      }}>{formatAmount(ask.amount)}</span>
                      <span className="total">{formatNumber(ask.total)}</span>
                    </div>
                  ))
                ) : (
                  <div className="empty-orderbook">
                    <span>N/A</span>
                  </div>
                )}
              </div>

              <div className="spread-indicator">
                <span className="spread-label">Spread</span>
                <span className="spread-value">
                  {spread.value !== 'N/A' ? `$${spread.value}` : 'N/A'}
                </span>
                <span className="spread-percentage">
                  {spread.percentage !== 'N/A' ? `(${spread.percentage})` : ''}
                </span>
              </div>

              <div className="bids-section">
                {orderbookData.bids.length > 0 ? (
                  orderbookData.bids.map((bid, index) => (
                    <div
                      key={index}
                      className="order-row bid"
                      onClick={() => handlePriceClick(bid.price)}
                    >
                      <div
                        className="depth-bar bid-depth"
                        style={{ width: `${bid.percentage || 0}%` }}
                      />
                      <span className="price bid-price">{formatPrice(bid.price)}</span>
                      <span className="amount" onClick={(e) => {
                        e.stopPropagation();
                        handleAmountClick(bid.amount);
                      }}>{formatAmount(bid.amount)}</span>
                      <span className="total">{formatNumber(bid.total)}</span>
                    </div>
                  ))
                ) : (
                  <div className="empty-orderbook">
                    <span>N/A</span>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>

        <div className="orderbook-center">
          <div className="chart-placeholder">
            <div className="chart-title">Price Chart</div>
            <div className="chart-content">
              <div className="chart-info">
                N/A - Chart data unavailable
              </div>
            </div>
          </div>

          <div className="recent-trades">
            <div className="trades-title">Recent Trades</div>
            <div className="trades-list">
              {recentTrades.length > 0 ? (
                recentTrades.map((trade, index) => (
                  <div key={index} className="trade-item">
                    <span className={`trade-price ${trade.side}`}>
                      {formatPrice(trade.price)}
                    </span>
                    <span className="trade-amount">
                      {formatAmount(trade.amount)}
                    </span>
                    <span className="trade-time">
                      {trade.timestamp}
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty-trades">
                  <span>N/A</span>
                </div>
              )}
            </div>
          </div>
        </div>

        <div className="orderbook-right">
          <div className="order-form">
            <div className="form-tabs">
              <button
                className={`form-tab ${orderSide === 'buy' ? 'active buy-tab' : ''}`}
                onClick={() => setOrderSide('buy')}
              >
                Buy
              </button>
              <button
                className={`form-tab ${orderSide === 'sell' ? 'active sell-tab' : ''}`}
                onClick={() => setOrderSide('sell')}
              >
                Sell
              </button>
            </div>

            <div className="order-types">
              <button
                className={`order-type-btn ${orderType === 'limit' ? 'active' : ''}`}
                onClick={() => setOrderType('limit')}
              >
                Limit
              </button>
              <button
                className={`order-type-btn ${orderType === 'market' ? 'active' : ''}`}
                onClick={() => setOrderType('market')}
              >
                Market
              </button>
            </div>

            <div className="form-inputs">
              {orderType === 'limit' && (
                <div className="input-group">
                  <label>Price</label>
                  <div className="input-wrapper">
                    <span className="input-label">Price</span>
                    <input
                      type="text"
                      value={price}
                      onChange={(e) => setPrice(e.target.value)}
                      placeholder="0.00"
                    />
                    <span className="input-suffix">USDC</span>
                  </div>
                </div>
              )}

              <div className="input-group">
                <label>Amount</label>
                <div className="input-wrapper">
                  <span className="input-label">Amount</span>
                  <input
                    type="text"
                    value={amount}
                    onChange={(e) => setAmount(e.target.value)}
                    placeholder="0.00"
                  />
                  <span className="input-suffix">BTC</span>
                </div>
              </div>

              <div className="percentage-buttons">
                <button onClick={() => setAmount('0')}>25%</button>
                <button onClick={() => setAmount('0')}>50%</button>
                <button onClick={() => setAmount('0')}>75%</button>
                <button onClick={() => setAmount('0')}>100%</button>
              </div>

              {price && amount && (
                <div className="order-summary">
                  <div className="summary-item">
                    <span>Total</span>
                    <span>{(parseFloat(price) * parseFloat(amount)).toFixed(2)} USDC</span>
                  </div>
                  <div className="summary-item">
                    <span>Fee (0.1%)</span>
                    <span>{(parseFloat(price) * parseFloat(amount) * 0.001).toFixed(4)} USDC</span>
                  </div>
                </div>
              )}

              <button
                className={`submit-order ${orderSide}`}
                onClick={handleSubmitOrder}
              >
                {orderSide === 'buy' ? 'Buy BTC' : 'Sell BTC'}
              </button>
            </div>

            <div className="balance-info">
              <div className="balance-item">
                <span>Available</span>
                <span>
                  {orderSide === 'buy'
                    ? (balance.usdc !== null ? `${formatNumber(balance.usdc, 2)} USDC` : 'N/A')
                    : (balance.btc !== null ? `${formatAmount(balance.btc)} BTC` : 'N/A')
                  }
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default OrderbookInterface;