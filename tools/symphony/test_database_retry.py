import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch, MagicMock

spec=importlib.util.spec_from_file_location('launcher',Path(__file__).parent/'environment/client/run.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class RetryTests(unittest.TestCase):
    def test_transient_failure_recovers_before_returning_connection(self):
        connection=object()
        with patch.object(m,'connect_once',side_effect=[ConnectionRefusedError('refused'),TimeoutError('slow'),connection]) as call, patch.object(m.time,'sleep'):
            self.assertIs(m.connect('172.30.2.2'),connection)
            self.assertEqual(call.call_count,3)

    def test_policy_denial_is_immediate(self):
        with patch.object(m,'connect_once',side_effect=PermissionError('denied')) as call:
            with self.assertRaises(PermissionError):m.connect('172.30.2.2')
            self.assertEqual(call.call_count,1)

    def test_permanent_outage_is_bounded(self):
        clock=[0.0]
        def sleep(seconds):clock[0]+=seconds
        with patch.object(m.time,'monotonic',side_effect=lambda:clock[0]),patch.object(m.time,'sleep',side_effect=sleep),patch.object(m,'connect_once',side_effect=ConnectionRefusedError('refused')):
            with self.assertRaisesRegex(TimeoutError,'unavailable after'):
                m.connect('172.30.2.2',timeout=1)
        self.assertEqual(clock[0],1)

    def test_socks_policy_denial_closes_socket_without_retry(self):
        sock=MagicMock()
        with patch.dict(m.os.environ,{'ALL_PROXY':'socks5://127.0.0.1:9999'}),patch.object(m.socket,'create_connection',return_value=sock),patch.object(m,'exact',side_effect=[b'\x05\x00',b'\x05\x02\x00\x01']):
            with self.assertRaises(PermissionError):m.connect('172.30.2.2')
        sock.close.assert_called_once()

    def test_socks_refusal_is_retryable_and_closes_socket(self):
        sock=MagicMock()
        with patch.dict(m.os.environ,{'ALL_PROXY':'socks5://127.0.0.1:9999'}),patch.object(m.socket,'create_connection',return_value=sock),patch.object(m,'exact',side_effect=[b'\x05\x00',b'\x05\x05\x00\x01']):
            with self.assertRaises(ConnectionRefusedError):m.connect_once('172.30.2.2',5)
        sock.close.assert_called_once()

    def test_failed_command_is_never_replayed(self):
        with patch.object(m,'connect',return_value=MagicMock()),patch.object(m,'Server'),patch.object(m.threading,'Thread'),patch.object(m.subprocess,'call',return_value=7) as call,patch.object(m.sys,'argv',['run.py','false']):
            self.assertEqual(m.main(),7)
            call.assert_called_once()

if __name__=='__main__':unittest.main()
