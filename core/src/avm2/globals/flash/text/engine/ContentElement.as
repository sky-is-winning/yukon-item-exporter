package flash.text.engine {
    import flash.events.EventDispatcher;
    public class ContentElement {
        public static const GRAPHIC_ELEMENT:uint = 65007;
        public var userData;
        
        private var _elementFormat:ElementFormat;
        
        public function ContentElement(elementFormat:ElementFormat = null, eventMirror:EventDispatcher = null, textRotation:String = "rotate0") {
            // FIXME: `new ContentElement()` throws an error in Flash; see TextJustifier
            this._elementFormat = elementFormat;
        }
        
        public function get elementFormat():ElementFormat {
            return this._elementFormat;
        }
        
        public function set elementFormat(value:ElementFormat):void {
            this._elementFormat = value;
        }
    }
}
