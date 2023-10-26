package flash.xml
{

    import __ruffle__.stub_getter;
    import __ruffle__.stub_method;

    import flash.xml.XMLNode;
    import flash.xml.XMLNodeType;

    public class XMLNode {
        internal var _children: Array = [];

        public var nodeType:uint;

        public var attributes:Object = {};

        public var nodeName:String = null;
        public var nodeValue:String = null;

        // [NA] parentNode, xChild and xSibling are settable in Flash. It makes no sense though and 100% would break things.
        // Oh well. Less work for us.
        public var parentNode:XMLNode = null;

        public var firstChild:XMLNode = null;
        public var lastChild:XMLNode = null;

        public var previousSibling:XMLNode = null;
        public var nextSibling:XMLNode = null;

        public function XMLNode(type: uint, input: String) {
            nodeType = type;
            if (type == XMLNodeType.ELEMENT_NODE) {
                nodeName = input;
            } else {
                nodeValue = input;
            }
        }

        public function get childNodes(): Array {
            return _children;
        }

        public function hasChildNodes() : Boolean
        {
            return _children.length > 0;
        }

        public function cloneNode() : XMLNode
        {
            stub_method("flash.xml.XMLNode", "cloneNode");
            return null;
        }

        public function removeNode() : void
        {
            stub_method("flash.xml.XMLNode", "removeNode");
        }

        public function insertBefore(node: XMLNode, before: XMLNode = null): void {
            if (before == null) {
                appendChild(node);
                return;
            }

            if (before.previousSibling != null) {
                // inserting in the middle
                before.previousSibling.nextSibling = node;

                for (var i = 0; i < childNodes.length; i++) {
                    if (childNodes[i] === before) {
                        childNodes.splice(i, 0, node);
                        break;
                    }
                }
            } else {
                // inserting at the start
                firstChild = node;
            }

            node.previousSibling = before.previousSibling;
            before.previousSibling = node;
            node.nextSibling = before;
            node.parentNode = this;
        }

        public function appendChild(node: XMLNode): void {
            if (node.parentNode === this) {
                return;
            }
            if (lastChild != null) {
                lastChild.nextSibling = node;
                node.previousSibling = lastChild;
            } else {
                firstChild = node;
                node.previousSibling = null;
            }
            node.nextSibling = null;
            node.parentNode = this;
            lastChild = node;

            _children.push(node);
        }

        public function getNamespaceForPrefix(prefix: String): String {
            stub_method("flash.xml.XMLNode", "getNamespaceForPrefix");
            return "";
        }

        public function getPrefixForNamespace(ns: String): String {
            stub_method("flash.xml.XMLNode", "getPrefixForNamespace");
            return "";
        }

        public function get localName(): String {
            if (nodeName == null) {
                return null;
            }
            var index = nodeName.indexOf(":");
            if (index > -1) {
                return nodeName.substring(index + 1);
            } else {
                return nodeName;
            }
        }

        public function get prefix(): String {
            if (nodeName == null) {
                return null;
            }
            var index = nodeName.indexOf(":");
            if (index > -1) {
                return nodeName.substring(0, index);
            } else {
                return "";
            }
        }

        public function get namespaceURI(): String {
            stub_getter("flash.xml.XMLNode", "namespaceURI");
            return null;
        }

        public function toString(): String {
            if (nodeType != XMLNodeType.ELEMENT_NODE) {
                return _escapeXML(nodeValue);
            }

            var result = "";
            if (nodeName != null) {
                result += "<" + nodeName;
            }

            for (var key in this.attributes) {
                result += " " + key + "=\"" + _escapeXML(this.attributes[key]) + "\"";
            }

            if (hasChildNodes()) {
                if (nodeName != null) {
                    result += ">";
                }
                for each (var child in childNodes) {
                    result += child.toString();
                }
                if (nodeName != null) {
                    result += "</" + nodeName + ">";
                }
            } else if (nodeName != null) {
                result += " />";
            }

            return result;
        }

        private native static function _escapeXML(text: String): String;

        internal function clear(): void {
            _children = [];

            attributes = {};

            parentNode = null;

            firstChild = null;
            lastChild = null;

            previousSibling = null;
            nextSibling = null;
        }
    }
}

