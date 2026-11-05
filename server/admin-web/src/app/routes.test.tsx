import { describe, expect, it } from 'vitest';
import {
  buildViewFromPath,
  navKeyFromPath,
  routeKeyFromPath,
  settingsViewFromPath
} from './routes';

describe('routeKeyFromPath', () => {
  it('keeps developer account and store workspace paths inside the React shell', () => {
    expect(routeKeyFromPath('/admin/accounts')).toBe('accounts');
    expect(routeKeyFromPath('/admin/accounts/account-ios')).toBe('accounts');
    expect(routeKeyFromPath('/admin/accounts/account-ios/apps/app-ios/store')).toBe('accounts');
    expect(routeKeyFromPath('/admin/accounts/account-ios/apps/app-ios/marketing')).toBe('accounts');
    expect(routeKeyFromPath('/admin/accounts/account-ios/apps/app-ios/marketing-pages/page-1')).toBe('accounts');
  });

  it('groups build and settings child paths under their workspaces', () => {
    expect(routeKeyFromPath('/admin/builds/apps')).toBe('builds');
    expect(routeKeyFromPath('/admin/builds/runners')).toBe('builds');
    expect(routeKeyFromPath('/admin/settings/llm')).toBe('settings');
    expect(routeKeyFromPath('/admin/settings/runtime')).toBe('settings');
  });

  it('does not keep removed top-level pages as compatibility routes', () => {
    expect(routeKeyFromPath('/admin/build-runners')).toBe('not-found');
    expect(routeKeyFromPath('/admin/llm-config')).toBe('not-found');
    expect(routeKeyFromPath('/admin/unknown')).toBe('not-found');
  });

  it('rejects unknown build and settings child paths', () => {
    expect(routeKeyFromPath('/admin/builds/unknown')).toBe('not-found');
    expect(routeKeyFromPath('/admin/builds/apps/extra')).toBe('not-found');
    expect(routeKeyFromPath('/admin/settings/unknown')).toBe('not-found');
    expect(routeKeyFromPath('/admin/settings/general/extra')).toBe('not-found');
  });
});

describe('buildViewFromPath', () => {
  it('defaults to build applications and resolves build child views', () => {
    expect(buildViewFromPath('/admin/builds')).toBe('apps');
    expect(buildViewFromPath('/admin/builds/apps')).toBe('apps');
    expect(buildViewFromPath('/admin/builds/history')).toBe('history');
    expect(buildViewFromPath('/admin/builds/runners')).toBe('runners');
  });
});

describe('settingsViewFromPath', () => {
  it('defaults to general settings and resolves settings child views', () => {
    expect(settingsViewFromPath('/admin/settings')).toBe('general');
    expect(settingsViewFromPath('/admin/settings/general')).toBe('general');
    expect(settingsViewFromPath('/admin/settings/notifications')).toBe('notifications');
    expect(settingsViewFromPath('/admin/settings/llm')).toBe('llm');
    expect(settingsViewFromPath('/admin/settings/runtime')).toBe('runtime');
  });
});

describe('navKeyFromPath', () => {
  it('highlights store management for app store workspace pages', () => {
    expect(navKeyFromPath('/admin/apps/app-ios-demo')).toBe('apps');
    expect(navKeyFromPath('/admin/accounts/account-ios/apps/app-ios/store')).toBe('apps');
    expect(navKeyFromPath('/admin/accounts/account-ios/apps/app-ios/marketing')).toBe('apps');
    expect(navKeyFromPath('/admin/accounts/account-ios/apps/app-ios/connection')).toBe('apps');
  });
});
