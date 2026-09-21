import { HttpTestingController } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { Auth } from '../src/app/auth';

// HTTP unit fixture: exercise session restoration before guarded navigation.
// Browser and backend suites use real account bootstrap and login.
export async function restoreTestSession() {
  const ready = TestBed.inject(Auth).check();
  TestBed.inject(HttpTestingController).expectOne('/api/auth/session').flush({
    username: 'unit-operator',
    csrf_token: 'unit-session-proof',
  });
  await ready;
}
