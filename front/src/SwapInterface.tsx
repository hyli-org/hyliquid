import { useState, useRef, useEffect } from 'react';
import './SwapInterface.css';

interface Token {
  symbol: string;
  name: string;
  balance: number;
  icon: string;
  price: number; // Price in USD
}

interface SwapInterfaceProps {
  mockMode: boolean;
}

function SwapInterface({ mockMode }: SwapInterfaceProps) {
  // Initialize tokens with mock or empty data based on mode
  const getInitialTokens = (): Token[] => {
    if (mockMode) {
      return [
        { symbol: 'BTC', name: 'Bitcoin', balance: 0.25342187, icon: '₿', price: 45000 },
        { symbol: 'USDC', name: 'USD Coin', balance: 10000.50, icon: '$', price: 1 },
        { symbol: 'USDT', name: 'Tether', balance: 5000.25, icon: '₮', price: 1 },
      ];
    }
    return [
      { symbol: 'BTC', name: 'Bitcoin', balance: 0, icon: '₿', price: 0 },
      { symbol: 'USDC', name: 'USD Coin', balance: 0, icon: '$', price: 0 },
      { symbol: 'USDT', name: 'Tether', balance: 0, icon: '₮', price: 0 },
    ];
  };

  const [tokens, setTokens] = useState<Token[]>(getInitialTokens());
  const [sellAmount, setSellAmount] = useState<string>('');
  const [buyAmount, setBuyAmount] = useState<string>('');
  const [sellToken, setSellToken] = useState<Token>(tokens[0]);
  const [buyToken, setBuyToken] = useState<Token | null>(null);
  const [showSellDropdown, setShowSellDropdown] = useState(false);
  const [showBuyDropdown, setShowBuyDropdown] = useState(false);
  const [sellDropdownPosition, setSellDropdownPosition] = useState({ top: 0, left: 0, width: 0 });
  const [buyDropdownPosition, setBuyDropdownPosition] = useState({ top: 0, left: 0, width: 0 });
  const sellButtonRef = useRef<HTMLButtonElement>(null);
  const buyButtonRef = useRef<HTMLButtonElement>(null);

  // Update tokens when mock mode changes
  useEffect(() => {
    const newTokens = getInitialTokens();
    setTokens(newTokens);
    setSellToken(newTokens[0]);
    setBuyToken(null);
    setSellAmount('');
    setBuyAmount('');
  }, [mockMode]);

  const formatAmount = (amount: number): string => {
    // Remove unnecessary trailing zeros
    const formatted = amount.toString();
    if (formatted.includes('.')) {
      return parseFloat(formatted).toString();
    }
    return formatted;
  };

  const handleSellAmountChange = (value: string) => {
    // Only allow numbers and decimal point
    if (/^\d*\.?\d*$/.test(value) || value === '') {
      setSellAmount(value);
      // Calculate conversion based on actual prices
      if (sellToken && buyToken && value && value !== '' && sellToken.price > 0 && buyToken.price > 0) {
        const sellValue = parseFloat(value) * sellToken.price;
        const convertedAmount = sellValue / buyToken.price;
        // Use appropriate decimal places based on token
        const decimals = buyToken.symbol === 'BTC' ? 8 : 2;
        setBuyAmount(formatAmount(parseFloat(convertedAmount.toFixed(decimals))));
      } else {
        setBuyAmount('');
      }
    }
  };

  const handleBuyAmountChange = (value: string) => {
    // Only allow numbers and decimal point
    if (/^\d*\.?\d*$/.test(value) || value === '') {
      setBuyAmount(value);
      // Calculate conversion based on actual prices
      if (sellToken && buyToken && value && value !== '' && sellToken.price > 0 && buyToken.price > 0) {
        const buyValue = parseFloat(value) * buyToken.price;
        const convertedAmount = buyValue / sellToken.price;
        // Use appropriate decimal places based on token
        const decimals = sellToken.symbol === 'BTC' ? 8 : 2;
        setSellAmount(formatAmount(parseFloat(convertedAmount.toFixed(decimals))));
      } else {
        setSellAmount('');
      }
    }
  };

  const handleSwapTokens = () => {
    const tempToken = sellToken;
    const tempAmount = sellAmount;
    const defaultBuyToken = tokens.find(t => t.symbol !== sellToken.symbol) || tokens[1];
    setSellToken(buyToken || defaultBuyToken);
    setBuyToken(tempToken);
    setSellAmount(buyAmount);
    setBuyAmount(tempAmount);
  };

  useEffect(() => {
    if (showSellDropdown && sellButtonRef.current) {
      const rect = sellButtonRef.current.getBoundingClientRect();
      setSellDropdownPosition({
        top: rect.bottom + 12,
        left: rect.left,
        width: Math.max(280, rect.width)
      });
    }
  }, [showSellDropdown]);

  useEffect(() => {
    if (showBuyDropdown && buyButtonRef.current) {
      const rect = buyButtonRef.current.getBoundingClientRect();
      setBuyDropdownPosition({
        top: rect.bottom + 12,
        left: rect.left,
        width: Math.max(280, rect.width)
      });
    }
  }, [showBuyDropdown]);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (showSellDropdown || showBuyDropdown) {
        const target = event.target as HTMLElement;
        if (!target.closest('.token-selector-wrapper') && !target.closest('.token-dropdown')) {
          setShowSellDropdown(false);
          setShowBuyDropdown(false);
        }
      }
    };

    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [showSellDropdown, showBuyDropdown]);

  const handleSelectSellToken = (token: Token) => {
    setSellToken(token);
    setShowSellDropdown(false);
    // Reset buy token if it's the same
    if (buyToken?.symbol === token.symbol) {
      setBuyToken(null);
      setBuyAmount('');
    } else if (buyToken && sellAmount && sellAmount !== '' && token.price > 0 && buyToken.price > 0) {
      // Recalculate with new sell token
      const sellValue = parseFloat(sellAmount) * token.price;
      const convertedAmount = sellValue / buyToken.price;
      const decimals = buyToken.symbol === 'BTC' ? 8 : 2;
      setBuyAmount(formatAmount(parseFloat(convertedAmount.toFixed(decimals))));
    }
  };

  const handleSelectBuyToken = (token: Token) => {
    setBuyToken(token);
    setShowBuyDropdown(false);
    // Trigger conversion with new token
    if (sellAmount && sellAmount !== '' && sellToken.price > 0 && token.price > 0) {
      // Recalculate with new buy token
      const sellValue = parseFloat(sellAmount) * sellToken.price;
      const convertedAmount = sellValue / token.price;
      const decimals = token.symbol === 'BTC' ? 8 : 2;
      setBuyAmount(formatAmount(parseFloat(convertedAmount.toFixed(decimals))));
    }
  };

  return (
    <div className="swap-container">
      <div className="swap-card">

        <div className="swap-section">
          <label className="section-label">Sell</label>
          <div className="input-group">
            <input
              type="text"
              className="amount-input"
              value={sellAmount}
              onChange={(e) => handleSellAmountChange(e.target.value)}
              placeholder="0"
            />
            <div className="token-selector-wrapper">
              <button
                ref={sellButtonRef}
                className="token-selector"
                onClick={() => {
                  setShowSellDropdown(!showSellDropdown);
                  setShowBuyDropdown(false);
                }}
              >
                <span className="token-icon">{sellToken.icon}</span>
                <span className="token-symbol">{sellToken.symbol}</span>
                <span className="dropdown-arrow">▼</span>
              </button>
            </div>
          </div>
          <div className="balance-display">
            <span className="balance-label">Balance: {sellToken.balance > 0 ? `${sellToken.balance} ${sellToken.symbol}` : 'N/A'}</span>
          </div>
        </div>

        <div className="swap-divider">
          <button className="swap-button" onClick={handleSwapTokens}>
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
              <path d="M12 5V19M12 19L7 14M12 19L17 14" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
          </button>
        </div>

        <div className="swap-section">
          <label className="section-label">Buy</label>
          <div className="input-group">
            <input
              type="text"
              className="amount-input"
              value={buyAmount}
              onChange={(e) => handleBuyAmountChange(e.target.value)}
              placeholder="0"
            />
            <div className="token-selector-wrapper">
              {buyToken ? (
                <button
                  ref={buyButtonRef}
                  className="token-selector"
                  onClick={() => {
                    setShowBuyDropdown(!showBuyDropdown);
                    setShowSellDropdown(false);
                  }}
                >
                  <span className="token-icon">{buyToken.icon}</span>
                  <span className="token-symbol">{buyToken.symbol}</span>
                  <span className="dropdown-arrow">▼</span>
                </button>
              ) : (
                <button
                  ref={buyButtonRef}
                  className="token-selector select-token"
                  onClick={() => {
                    setShowBuyDropdown(!showBuyDropdown);
                    setShowSellDropdown(false);
                  }}
                >
                  Select token
                  <span className="dropdown-arrow">▼</span>
                </button>
              )}
            </div>
          </div>
          {buyToken && (
            <div className="balance-display">
              <span className="balance-label">Balance: {buyToken.balance > 0 ? `${buyToken.balance} ${buyToken.symbol}` : 'N/A'}</span>
            </div>
          )}
        </div>

        <button
          className="swap-submit-button"
          disabled={!buyToken || sellAmount === '' || parseFloat(sellAmount) <= 0}
        >
          {!buyToken ? 'Select a token to buy' : 'Get started'}
        </button>
      </div>

      {showSellDropdown && (
        <div
          className="token-dropdown"
          style={{
            top: `${sellDropdownPosition.top}px`,
            left: `${sellDropdownPosition.left}px`,
            width: `${sellDropdownPosition.width}px`
          }}
        >
          {tokens.map((token) => (
            <div
              key={token.symbol}
              className="token-option"
              onClick={() => handleSelectSellToken(token)}
            >
              <span className="token-icon">{token.icon}</span>
              <div className="token-info">
                <span className="token-symbol">{token.symbol}</span>
                <span className="token-name">{token.name}</span>
              </div>
              <span className="token-balance">{token.balance}</span>
            </div>
          ))}
        </div>
      )}

      {showBuyDropdown && (
        <div
          className="token-dropdown"
          style={{
            top: `${buyDropdownPosition.top}px`,
            left: `${buyDropdownPosition.left}px`,
            width: `${buyDropdownPosition.width}px`
          }}
        >
          {tokens
            .filter(token => token.symbol !== sellToken.symbol)
            .map((token) => (
            <div
              key={token.symbol}
              className="token-option"
              onClick={() => handleSelectBuyToken(token)}
            >
              <span className="token-icon">{token.icon}</span>
              <div className="token-info">
                <span className="token-symbol">{token.symbol}</span>
                <span className="token-name">{token.name}</span>
              </div>
              <span className="token-balance">{token.balance}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default SwapInterface;
