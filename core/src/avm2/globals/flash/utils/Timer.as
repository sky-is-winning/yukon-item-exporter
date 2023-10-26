package flash.utils {
	import flash.events.EventDispatcher;
	import flash.events.TimerEvent;
	public class Timer extends EventDispatcher {
		internal var _currentCount: int;
		internal var _delay: Number;
		internal var _repeatCount: int;
		internal var _timerId: int = -1;

		private function checkDelay(delay:Number): void {
			if (!isFinite(delay) || delay < 0) {
				throw new RangeError("Timer delay out of range", 2066);
			}
		}

		public function Timer(delay:Number, repeatCount:int=0) {
			this.checkDelay(delay);
			this._currentCount = 0;
			this._delay = delay;
			this._repeatCount = repeatCount;
		}

		public function get currentCount(): int {
			return this._currentCount;
		}

		public function get delay(): Number {
			return this._delay;
		}

		public function set delay(value:Number): void {
			this.checkDelay(delay);
			this._delay = value;
			if (this.running) {
				this.stop();
				this.start();
			}
		}

		public function get repeatCount(): int {
			return this._repeatCount;
		}

		public function set repeatCount(value:int): void {
			this._repeatCount = value;
			if (this._repeatCount != 0 && this._repeatCount <= this._currentCount) {
				this.stop();
			}
		}

		public function get running(): Boolean {
			return this._timerId != -1;
		}

		public function reset():void {
			this._currentCount = 0;
			this.stop();
		}

		public native function stop():void;
		public native function start():void;

		// Returns 'true' if we should cancel the underlying Ruffle native timer
		internal function onUpdate():Boolean {
			this._currentCount += 1;
			this.dispatchEvent(new TimerEvent(TimerEvent.TIMER, false, false));
			if (this.repeatCount != 0 && this._currentCount >= this._repeatCount) {
				// This will make 'running' return false in a TIMER_COMPLETE event handler
				this._timerId = -1;
				this.dispatchEvent(new TimerEvent(TimerEvent.TIMER_COMPLETE, false, false));
				return true;
			}
			return false;
		}
	}
}
