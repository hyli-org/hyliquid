import { useState, useEffect } from 'react';
import SwapInterface from './SwapInterface';
import OrderbookInterface from './OrderbookInterface';
import './App.css';

function App() {
  const [viewMode, setViewMode] = useState<'simple' | 'pro'>('simple');
  const [darkMode, setDarkMode] = useState(() => {
    const saved = localStorage.getItem('darkMode');
    return saved === 'true';
  });
  const [mockMode, setMockMode] = useState(() => {
    // Check environment variable first, then localStorage
    const envDefault = import.meta.env.VITE_MOCK_MODE_DEFAULT === 'true';
    const saved = localStorage.getItem('mockMode');
    return saved !== null ? saved === 'true' : envDefault;
  });

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', darkMode ? 'dark' : 'light');
    localStorage.setItem('darkMode', String(darkMode));
  }, [darkMode]);

  useEffect(() => {
    localStorage.setItem('mockMode', String(mockMode));
    // Add mock mode indicator to document
    document.documentElement.setAttribute('data-mock', String(mockMode));
  }, [mockMode]);

  return (
    <div className="app-container">
      <div className={`view-tabs ${viewMode === 'pro' ? 'pro-mode' : ''}`}>
        <button
          className={`view-tab ${viewMode === 'simple' ? 'active' : ''}`}
          onClick={() => setViewMode('simple')}
        >
          Simple
        </button>
        <button
          className={`view-tab ${viewMode === 'pro' ? 'active' : ''}`}
          onClick={() => setViewMode('pro')}
        >
          Pro View
        </button>
        <button
          className="theme-toggle"
          onClick={() => setDarkMode(!darkMode)}
          aria-label="Toggle dark mode"
        >
          {darkMode ? '☀️' : '🌙'}
        </button>
        <button
          className={`mock-toggle ${mockMode ? 'active' : ''}`}
          onClick={() => setMockMode(!mockMode)}
          aria-label="Toggle mock data"
          title={mockMode ? 'Mock data ON' : 'Mock data OFF'}
        >
          {mockMode ? '🔌' : '⚡'}
        </button>
      </div>

      {mockMode && (
        <div className="mock-indicator">
          Mock Data Mode
        </div>
      )}

      {viewMode === 'simple' ? <SwapInterface mockMode={mockMode} /> : <OrderbookInterface mockMode={mockMode} />}
    </div>
  );
}

export default App;