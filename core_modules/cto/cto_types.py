# ~/aurora_project/core_modules/cto/cto_types.py
import abc
from typing import TypeVar, Generic, Callable, Any, Set, Type

# Define Type Variables for generics
D = TypeVar('D') # Domain Type (the type of the data that goes into the operation)
C = TypeVar('C') # Codomain Type (the type of the data that comes out of the operation)

ObjValue = TypeVar('ObjValue') # Any type for the *value* encapsulated by an Object

class Object(Generic[ObjValue], abc.ABC):
    """
    Abstract Base Class for an Object in a Category.
    In AURORA, an Object represents a type of computational entity, data structure,
    or system state. Its generic type parameter `ObjValue` is the *Python type*
    that instances of this object will represent or encapsulate (e.g., int, str,
    MyCustomDataclass).
    """
    def __init__(self, value_type: Type[ObjValue]):
        self._value_type = value_type

    @property
    def value_type(self) -> Type[ObjValue]:
        return self._value_type

    @abc.abstractmethod
    def identity(self) -> 'Morphism[ObjValue, ObjValue]':
        """
        Returns the identity morphism for this object's value_type.
        The identity morphism maps any instance of the object's value_type to itself.
        """
        pass

    def __str__(self) -> str:
        return f"Object({self.value_type.__name__})"

    def __repr__(self) -> str:
        return self.__str__()

    def __eq__(self, other: Any) -> bool:
        if not isinstance(other, Object):
            return NotImplemented
        return self.value_type == other.value_type

    def __hash__(self) -> int:
        return hash(self.value_type)


class Morphism(Generic[D, C], abc.ABC):
    """
    Abstract Base Class for a Morphism in a Category.
    In AURORA, a Morphism represents any computation, transformation,
    API call, or interaction between instances of computational Objects.
    It maps from an instance of `D` type to an instance of `C` type.
    """
    def __init__(self, domain_object: 'Object[D]', codomain_object: 'Object[C]', operation: Callable[[D], C]):
        self._domain_object = domain_object
        self._codomain_object = codomain_object
        self._operation = operation

        # Pylance was reporting "UnnecessaryIsInstance" for isinstance(operation, Callable).
        # We can remove this runtime check as type hints handle it statically in strict mode.
        # However, for robustness in a complex system where inputs might bypass type checkers,
        # it might be re-introduced as a runtime assert or check in a production system.

    @property
    def domain(self) -> 'Object[D]':
        """The domain object of this morphism (representing the input type)."""
        return self._domain_object

    @property
    def codomain(self) -> 'Object[C]':
        """The codomain object of this morphism (representing the output type)."""
        return self._codomain_object

    @abc.abstractmethod # Re-add abstract method to force concrete subclasses to implement
    def __call__(self, input_data: D) -> C:
        """
        Applies the operation of the morphism to an instance of the input type.
        This is the core execution logic of the transformation.
        """
        pass # This is abstract, concrete classes will implement it


    def __str__(self) -> str:
        return f"Morphism({self.domain.value_type.__name__} -> {self.codomain.value_type.__name__})"

    def __repr__(self) -> str:
        return self.__str__()


class Category(Generic[ObjValue, D, C], abc.ABC): # Refined generics for clarity in Category
    """
    Abstract Base Class for a Category.
    A Category in AURORA defines a collection of Objects (representing types)
    and Morphisms (transformations between instances of those types),
    adhering to specific compositional rules.
    """
    @abc.abstractmethod
    def objects(self) -> Set['Object[Any]']: # Use string literal for forward reference
        """Returns the set of all objects (representing types) in this category."""
        pass

    @abc.abstractmethod
    def morphisms(self, domain: 'Object[Any]', codomain: 'Object[Any]') -> Set['Morphism[Any, Any]']:
        """
        Returns the set of all morphisms from domain type to codomain type.
        Parameter names `domain` and `codomain` must match the abstract method.
        """
        pass

    # The compose method is concrete in the abstract base class as it provides default behavior
    def compose(self, m1: 'Morphism[Any, Any]', m2: 'Morphism[Any, Any]') -> 'Morphism[Any, Any]':
        """
        Composes two morphisms m1 and m2 such that m1;m2 (m2 after m1).
        Requires m1.codomain == m2.domain.
        Returns a new morphism representing the sequential application.
        """
        if m1.codomain != m2.domain:
            raise ValueError(f"Cannot compose morphisms: codomain of m1 ({m1.codomain}) does not match domain of m2 ({m2.domain})")

        # Define the composed operation dynamically
        def composed_operation(input_data: Any) -> Any:
            # First apply m1, then apply m2 to the result
            intermediate_result = m1(input_data)
            return m2(intermediate_result)

        # Create a new BasicMorphism representing the composition
        return BasicMorphism(m1.domain, m2.codomain, composed_operation)

    def __str__(self) -> str:
        return f"Category({self.__class__.__name__})"

    def __repr__(self) -> str:
        return self.__str__()


# --- Basic Concrete Implementations for illustration ---

class BasicObject(Object[ObjValue]):
    """A concrete implementation of an Object, representing a specific Python type."""
    def __init__(self, value_type: Type[ObjValue]):
        # BasicObject simply encapsulates a Python type (e.g., int, str, bool)
        super().__init__(value_type)

    def identity(self) -> 'Morphism[ObjValue, ObjValue]':
        """
        Returns the identity morphism for this BasicObject.
        The identity morphism for a given type simply returns its input instance unchanged.
        """
        # Identity operation takes an instance of ObjValue and returns it
        return BasicMorphism(self, self, lambda x: x)


class BasicMorphism(Morphism[D, C]):
    """
    A concrete implementation of a Morphism for basic function application.
    This class *must* implement the abstract `__call__` method from its parent `Morphism`.
    """
    def __init__(self, domain_object: 'Object[D]', codomain_object: 'Object[C]', operation: Callable[[D], C]):
        super().__init__(domain_object, codomain_object, operation)

    def __call__(self, input_data: D) -> C:
        """
        Applies the operation stored in _operation.
        This explicitly implements the abstract __call__ method.
        """
        return self._operation(input_data)


# --- Example Usage (will be moved to tests.py later) ---
if __name__ == "__main__":
    # Define some objects representing Python types
    int_obj = BasicObject(int)
    str_obj = BasicObject(str)
    float_obj = BasicObject(float)

    # Define some operations (functions that work on *instances* of these types)
    def int_to_str(x: int) -> str:
        return str(x)

    def str_to_len(s: str) -> int:
        return len(s)

    def int_to_float_half(x: int) -> float:
        return float(x) / 2.0

    # Create morphisms
    m_int_to_str = BasicMorphism(int_obj, str_obj, int_to_str)
    m_str_to_len = BasicMorphism(str_obj, int_obj, str_to_len)
    m_int_to_float = BasicMorphism(int_obj, float_obj, int_to_float_half)

    # Apply a morphism to an instance of its domain type
    result_str = m_int_to_str(123)
    print(f"Applying m_int_to_str to 123: '{result_str}' (type: {type(result_str).__name__})")

    # Example of a concrete Category to test composition
    # The generic types for Category are ObjValue, MorphismInput (D), MorphismOutput (C)
    class MyBasicCategory(Category[Type[Any], Any, Any]):
        def objects(self) -> Set[Object[Any]]:
            return {int_obj, str_obj, float_obj} # Use defined BasicObject instances

        def morphisms(self, domain: Object[Any], codomain: Object[Any]) -> Set[Morphism[Any, Any]]:
            # Parameter names `domain` and `codomain` match the abstract method.
            # This is simplified; in a real system, you'd have a registry of morphisms
            found_morphisms: Set[Morphism[Any, Any]] = set()
            if domain == int_obj and codomain == str_obj:
                found_morphisms.add(m_int_to_str)
            if domain == str_obj and codomain == int_obj:
                found_morphisms.add(m_str_to_len)
            if domain == int_obj and codomain == float_obj:
                found_morphisms.add(m_int_to_float)
            return found_morphisms

    my_cat = MyBasicCategory()

    # Compose morphisms using the category's compose method
    m_int_to_len = my_cat.compose(m_int_to_str, m_str_to_len)
    final_result_int = m_int_to_len(4567) # Apply to an integer instance
    print(f"Composing m_int_to_str and m_str_to_len, then applying to 4567: {final_result_int} (type: {type(final_result_int).__name__})")

    # Test identity morphism
    identity_int_morphism = int_obj.identity()
    result_identity = identity_int_morphism(42)
    print(f"Identity of int_obj applied to 42: {result_identity} (should be 42, type: {type(result_identity).__name__})")

    identity_str_morphism = str_obj.identity()
    result_identity_str = identity_str_morphism("hello")
    print(f"Identity of str_obj applied to 'hello': {result_identity_str} (should be 'hello', type: {type(result_identity_str).__name__})")

    # Test invalid composition
    try:
        # Pylance was complaining about 'Literal[123]' etc.
        # Now that BasicMorphism.__call__ is implemented and typed,
        # and the example usage uses instances, these warnings should be gone.
        my_cat.compose(m_int_to_str, m_int_to_float) # str -> float cannot follow int -> str
    except ValueError as e:
        print(f"Caught expected error for invalid composition: {e}")