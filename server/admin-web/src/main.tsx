import React from 'react';
import { createRoot } from 'react-dom/client';
import { AdminApp } from './app/AdminApp';
import './styles/admin.css';

const root = document.getElementById('root');

if (!root) {
  throw new Error('Admin root element is missing');
}

createRoot(root).render(
  <React.StrictMode>
    <AdminApp />
  </React.StrictMode>
);
